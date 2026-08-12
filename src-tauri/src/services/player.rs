use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::Emitter;
use tauri::AppHandle;
use tracing::{debug, error, info, warn};

use crate::infra::repos::HistoryRepo;

/// Progreso guardado cada N segundos de reproducción.
const SAVE_INTERVAL_SECS: f64 = 10.0;
/// Frecuencia de polling del reproductor.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone)]
pub enum PlayerCommand {
    Play {
        slug: String,
        number: i32,
        url: String,
        title: String,
        start: f64,
    },
    Pause,
    Resume,
    TogglePause,
    Seek(f64),
    SetSpeed(f64),
    SetVolume(i64),
    ToggleMute,
    SetFullscreen(bool),
    SetSubtitles(Option<String>),
    Stop,
}

/// Fase de reproducción: representación explícita de la máquina de estados del
/// reproductor. Es la fuente de verdad para decidir si un evento de mpv
/// pertenece a la reproducción actual o a una anterior (obsoleta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerPhase {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Buffering,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PlayerState {
    /// Identidad de la sesión de reproducción activa. Cada `Play` crea una
    /// sesión nueva; `0` significa que no hay reproducción.
    pub session_id: u64,
    pub phase: PlayerPhase,
    pub loaded: bool,
    pub playing: bool,
    pub buffering: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub volume: i64,
    pub muted: bool,
    pub fullscreen: bool,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub number: i32,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
struct ProgressEvent {
    session_id: u64,
    slug: String,
    number: i32,
    position: f64,
    duration: f64,
}

/// Identidad de una reproducción. El id es monotónico y nunca se reutiliza, de
/// modo que los eventos de mpv pertenecientes a una sesión anterior pueden
/// detectarse y descartarse sin afectar a la sesión activa.
#[derive(Debug, Clone)]
struct PlaybackSession {
    id: u64,
    slug: String,
    number: i32,
}

/// Controlador del reproductor basado en libmpv.
///
/// libmpv se ejecuta en su propio hilo; los comandos llegan por canal y el
/// estado se comparte vía `Arc<Mutex>`. El hilo guarda progreso cada 10 s y
/// emite eventos Tauri para la interfaz.
pub struct PlayerService {
    tx: Sender<PlayerCommand>,
    state: Arc<Mutex<PlayerState>>,
}

impl PlayerService {
    pub fn spawn(app: AppHandle, pool: SqlitePool, rt: tauri::async_runtime::RuntimeHandle) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<PlayerCommand>();
        let state = Arc::new(Mutex::new(PlayerState::default()));

        let thread_state = state.clone();
        std::thread::Builder::new()
            .name("kakazuma-mpv".into())
            .spawn(move || {
                player_loop(rx, thread_state, app, pool, rt);
            })
            .map_err(|e| error!(error = %e, "no se pudo crear el hilo del reproductor"))
            .ok();

        Self { tx, state }
    }

    pub fn send(&self, cmd: PlayerCommand) {
        let _ = self.tx.send(cmd);
    }

    pub fn get_state(&self) -> PlayerState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Referencia compartida al estado del reproductor (para el buffer).
    pub fn state_handle(&self) -> Arc<Mutex<PlayerState>> {
        self.state.clone()
    }

    pub fn play(&self, slug: &str, number: i32, url: &str, title: &str, start: f64) {
        self.send(PlayerCommand::Play {
            slug: slug.to_string(),
            number,
            url: url.to_string(),
            title: title.to_string(),
            start,
        });
    }
}

fn player_loop(
    rx: Receiver<PlayerCommand>,
    state: Arc<Mutex<PlayerState>>,
    app: AppHandle,
    pool: SqlitePool,
    rt: tauri::async_runtime::RuntimeHandle,
) {
    unsafe {
        libc::setlocale(libc::LC_NUMERIC, "C\0".as_ptr().cast());
    }
    let mut mpv = match mpv::MpvHandlerBuilder::new() {
        Ok(builder) => {
            let mut builder = builder;
            let _ = builder.set_option("terminal", false);
            let _ = builder.set_option("input-default-bindings", true);
            let _ = builder.set_option("input-vo-keyboard", true);
            let _ = builder.set_option("osc", true);
            let _ = builder.set_option("force-window", "no");
            // Cache del demuxer para suavizar fluctuaciones de red del
            // episodio en curso (complementa al Smart Buffer).
            let _ = builder.set_option("cache", "yes");
            let _ = builder.set_option("cache-secs", 300i64);
            let _ = builder.set_option("demuxer-max-bytes", 536_870_912i64); // 512 MiB
            let _ = builder.set_option("demuxer-max-back-bytes", 134_217_728i64); // 128 MiB
            let _ = builder.set_option("demuxer-readahead-secs", 120i64);
            match builder.build() {
                Ok(m) => Box::new(m),
                Err(e) => {
                    let msg = format!("no se pudo inicializar libmpv: {e}");
                    set_error(&state, &app, &msg);
                    error!(%msg);
                    return;
                }
            }
        }
        Err(e) => {
            let msg = format!("no se pudo crear libmpv: {e}");
            set_error(&state, &app, &msg);
            error!(%msg);
            return;
        }
    };

    let _ = mpv.set_property("volume", 100i64);
    let _ = mpv.set_property("speed", 1.0f64);

    info!("libmpv listo");
    emit_state(&state, &app);

    let mut last_saved: f64 = 0.0;
    // Sesión de reproducción activa (la fuente de verdad del servicio).
    let mut session: Option<PlaybackSession> = None;
    // Contador monotónico de sesiones; nunca se reutilizan ids.
    let mut next_session_id: u64 = 0;
    // Mientras la sesión activa no haya producido su `StartFile`, cualquier
    // `EndFile` pertenece a un archivo anterior y debe ignorarse.
    let mut awaiting_start = true;

    loop {
        // Drenar comandos pendientes.
        loop {
            match rx.try_recv() {
                Ok(cmd) => {
                    apply_command(
                        &cmd,
                        &mut mpv,
                        &state,
                        &app,
                        &mut session,
                        &mut awaiting_start,
                        &mut next_session_id,
                        &mut last_saved,
                        &pool,
                        &rt,
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    debug!("canal del reproductor cerrado, deteniendo");
                    let _ = mpv.command(&["stop"]);
                    return;
                }
            }
        }

        // Poll de posición/duración.
        if let (Ok(pos), Ok(dur), Ok(pause), Ok(buffering)) = (
            mpv.get_property::<f64>("time-pos"),
            mpv.get_property::<f64>("duration"),
            mpv.get_property::<bool>("pause"),
            mpv.get_property::<bool>("paused-for-cache"),
        ) {
            let dur = dur.max(0.0);
            let pos = pos.max(0.0);
            if let Ok(mut s) = state.lock() {
                s.position = pos;
                s.duration = dur;
                if s.session_id != 0 {
                    match s.phase {
                        PlayerPhase::Playing | PlayerPhase::Paused | PlayerPhase::Buffering => {
                            s.playing = !pause && dur > 0.0;
                            s.loaded = dur > 0.0;
                            s.buffering = buffering;
                            s.phase = if buffering {
                                PlayerPhase::Buffering
                            } else if pause {
                                PlayerPhase::Paused
                            } else {
                                PlayerPhase::Playing
                            };
                        }
                        PlayerPhase::Loading | PlayerPhase::Stopping | PlayerPhase::Error => {}
                        PlayerPhase::Idle => {
                            s.playing = false;
                            s.buffering = buffering;
                        }
                    }
                } else {
                    s.playing = false;
                    s.buffering = buffering;
                }
            }
        }
        emit_state(&state, &app);

        // Guardar progreso cada 10 s (sin bloquear el hilo).
        if let Ok(s) = state.lock() {
            if s.session_id != 0 && s.duration > 0.0 && s.position - last_saved >= SAVE_INTERVAL_SECS {
                let slug = s.slug.clone().unwrap_or_default();
                let number = s.number;
                let position = s.position;
                let duration = s.duration;
                let session_id = s.session_id;
                save_progress(&pool, &rt, &slug, number, position, duration);
                let _ = app.emit(
                    "player://progress",
                    ProgressEvent {
                        session_id,
                        slug,
                        number,
                        position,
                        duration,
                    },
                );
                last_saved = position;
            }
        }

        // Eventos de mpv.
        while let Some(ev) = mpv.wait_event(0.0) {
            match ev {
                mpv::Event::StartFile => {
                    debug!("mpv: archivo iniciado");
                    let mut notify = false;
                    if let Ok(mut s) = state.lock() {
                        if s.phase == PlayerPhase::Loading && s.session_id != 0 {
                            // El archivo solicitado por la sesión activa está
                            // abriendo: a partir de aquí los EndFile dejan de
                            // ser obsoletos para esta sesión.
                            awaiting_start = false;
                            s.loaded = true;
                            notify = true;
                        }
                    }
                    if notify {
                        emit_state(&state, &app);
                    }
                }
                mpv::Event::FileLoaded => {
                    debug!("mpv: archivo cargado");
                    let mut seek_to: Option<f64> = None;
                    let mut notify = false;
                    if let Ok(mut s) = state.lock() {
                        if s.session_id != 0 {
                            awaiting_start = false;
                            if s.position > 5.0 {
                                seek_to = Some(s.position);
                            }
                            if s.phase == PlayerPhase::Loading {
                                s.error = None;
                                s.loaded = true;
                                s.playing = true;
                                s.phase = PlayerPhase::Playing;
                                notify = true;
                            }
                        }
                    }
                    if let Some(pos) = seek_to {
                        let _ = mpv.set_property("time-pos", pos);
                    }
                    if notify {
                        emit_state(&state, &app);
                    }
                }
                mpv::Event::EndFile(reason) => {
                    debug!("mpv: fin de archivo ({reason:?})");
                    handle_end_file(
                        reason,
                        &state,
                        &app,
                        &mut session,
                        &mut awaiting_start,
                        &pool,
                        &rt,
                    );
                }
                mpv::Event::Idle => {
                    handle_idle(
                        &state,
                        &app,
                        &mut session,
                        &mut awaiting_start,
                        &pool,
                        &rt,
                    );
                }
                _ => {}
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Clasifica un `EndFile` de mpv respecto a la sesión activa.
///
/// Un `EndFile` puede representar: EOF natural, reemplazo (`loadfile ... replace`),
/// `stop` explícito, error o cancelación. La regla fundamental es que un evento
/// perteneciente a una reproducción anterior nunca puede mutar la sesión actual:
///
/// * `EndFile(STOP)` nunca termina la sesión actual: solo aparece por un
///   `loadfile replace` (la sesión ya avanzó) o por un `stop` explícito (que se
///   confirma por separado mediante la fase `Stopping`).
/// * `EndFile(EOF)`/error mientras la sesión está en `Loading` y aún no se ha
///   visto su `StartFile` pertenece a un archivo anterior y se ignora.
/// * Los EOF/errores de un archivo anterior que jamás arrancó (p. ej. A termina
///   justo cuando se selecciona B) no pueden limpiar el estado de B.
fn handle_end_file(
    reason: mpv::Result<mpv::EndFileReason>,
    state: &Arc<Mutex<PlayerState>>,
    app: &AppHandle,
    session: &mut Option<PlaybackSession>,
    awaiting_start: &mut bool,
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
) {
    // Confirmación de un `Stop` explícito: se completa la limpieza.
    {
        let phase = state.lock().map(|s| s.phase).unwrap_or_default();
        if phase == PlayerPhase::Stopping {
            debug!("stop confirmado, limpiando estado");
            reset_to_idle(state, session, awaiting_start);
            emit_state(state, app);
            return;
        }
    }

    // Sin sesión activa: evento obsoleto (por ejemplo, el EndFile que genera un
    // stop de un archivo ya reemplazado).
    if session.is_none() {
        debug!("EndFile sin sesión activa: ignorado");
        return;
    }

    match reason {
        Ok(mpv::EndFileReason::MPV_END_FILE_REASON_STOP) => {
            // Reemplazo/cancelación: la sesión activa ya avanzó (o se detuvo
            // explícitamente, caso gestionado arriba). Ignorar siempre.
            debug!("EndFile(STOP) obsoleto (reemplazo/cancelación)");
        }
        Ok(mpv::EndFileReason::MPV_END_FILE_REASON_EOF) => {
            let phase = state.lock().map(|s| s.phase).unwrap_or_default();
            if phase == PlayerPhase::Loading {
                // A terminó por EOF justo cuando la nueva sesión carga (Caso 3).
                // El progreso de A ya se guardó al ejecutar Play(B).
                debug!("EndFile(EOF) obsoleto durante carga");
            } else {
                end_current_session(state, app, pool, rt, session, awaiting_start);
            }
        }
        Ok(reason) => {
            debug!("EndFile({reason:?}): fin de la sesión actual");
            end_current_session(state, app, pool, rt, session, awaiting_start);
        }
        Err(e) => {
            let phase = state.lock().map(|s| s.phase).unwrap_or_default();
            if phase == PlayerPhase::Loading {
                if *awaiting_start {
                    debug!(error = %e, "error de un archivo anterior durante carga: ignorado");
                } else {
                    // El archivo solicitado falló al cargar, o un archivo
                    // intermedio falló antes de que un reemplazo más nuevo
                    // tomara el control. Se reporta el error pero NO se
                    // destruye la sesión: si un archivo posterior sigue
                    // cargando, StartFile/FileLoaded continuarán; si no,
                    // `Idle` se encarga de la limpieza.
                    let msg = format!("no se pudo reproducir el episodio: {e}");
                    if let Ok(mut s) = state.lock() {
                        s.error = Some(msg.clone());
                    }
                    let _ = app.emit("player://error", msg);
                }
            } else {
                // Error durante la reproducción: fin legítimo de la sesión.
                error_end_current_session(state, app, pool, rt, session, &e.to_string());
            }
        }
    }
}

/// Finalización natural (EOF) de la sesión activa: guarda progreso, emite
/// `player://end` con la identidad de la sesión y limpia el estado. No ejecuta
/// `stop`: mpv cierra la ventana por sí mismo y emitirá `Idle`.
fn end_current_session(
    state: &Arc<Mutex<PlayerState>>,
    app: &AppHandle,
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
    session: &mut Option<PlaybackSession>,
    awaiting_start: &mut bool,
) {
    let Some(sess) = session.as_ref() else { return };
    let (session_id, slug, number) = (sess.id, sess.slug.clone(), sess.number);
    let (position, duration) = state
        .lock()
        .map(|s| (s.position, s.duration))
        .unwrap_or_default();
    if duration > 0.0 {
        save_progress(pool, rt, &slug, number, position, duration);
    }
    let _ = app.emit(
        "player://end",
        ProgressEvent {
            session_id,
            slug,
            number,
            position,
            duration,
        },
    );
    reset_to_idle(state, session, awaiting_start);
    emit_state(state, app);
    info!("reproducción terminada, estado limpiado");
}

/// La sesión activa terminó con error: guarda progreso, emite `player://end` y
/// `player://error`, y deja la fase en `Error` hasta que `Idle` confirme la
/// limpieza (mantiene la identidad de la sesión para los listeners).
fn error_end_current_session(
    state: &Arc<Mutex<PlayerState>>,
    app: &AppHandle,
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
    session: &mut Option<PlaybackSession>,
    message: &str,
) {
    let Some(sess) = session.as_ref() else { return };
    let (session_id, slug, number) = (sess.id, sess.slug.clone(), sess.number);
    let (position, duration) = state
        .lock()
        .map(|s| (s.position, s.duration))
        .unwrap_or_default();
    if duration > 0.0 {
        save_progress(pool, rt, &slug, number, position, duration);
    }
    let _ = app.emit(
        "player://end",
        ProgressEvent {
            session_id,
            slug,
            number,
            position,
            duration,
        },
    );
    if let Ok(mut s) = state.lock() {
        s.phase = PlayerPhase::Error;
        s.error = Some(message.to_string());
        s.loaded = false;
        s.playing = false;
    }
    let _ = app.emit("player://error", message.to_string());
    emit_state(state, app);
}

/// mpv quedó sin archivos. Resuelve la sesión activa pendiente (carga fallida o
/// cancelada) o, defensivamente, una reproducción que terminó sin EndFile.
fn handle_idle(
    state: &Arc<Mutex<PlayerState>>,
    app: &AppHandle,
    session: &mut Option<PlaybackSession>,
    awaiting_start: &mut bool,
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
) {
    let phase = state.lock().map(|s| s.phase).unwrap_or_default();
    if session.is_none() || phase == PlayerPhase::Idle {
        if let Ok(mut s) = state.lock() {
            s.playing = false;
        }
        emit_state(state, app);
        return;
    }
    match phase {
        PlayerPhase::Playing | PlayerPhase::Paused | PlayerPhase::Buffering => {
            end_current_session(state, app, pool, rt, session, awaiting_start);
        }
        _ => {
            reset_to_idle(state, session, awaiting_start);
            emit_state(state, app);
        }
    }
}

/// Devuelve el reproductor al estado `Idle`, invalidando la sesión activa. Los
/// eventos posteriores de mpv se descartan por no haber sesión.
fn reset_to_idle(
    state: &Arc<Mutex<PlayerState>>,
    session: &mut Option<PlaybackSession>,
    awaiting_start: &mut bool,
) {
    if let Ok(mut s) = state.lock() {
        s.phase = PlayerPhase::Idle;
        s.session_id = 0;
        s.loaded = false;
        s.playing = false;
        s.buffering = false;
        s.position = 0.0;
        s.duration = 0.0;
    }
    *session = None;
    *awaiting_start = true;
}

#[allow(clippy::too_many_arguments)]
fn apply_command(
    cmd: &PlayerCommand,
    mpv: &mut Box<mpv::MpvHandler>,
    state: &Arc<Mutex<PlayerState>>,
    app: &AppHandle,
    session: &mut Option<PlaybackSession>,
    awaiting_start: &mut bool,
    next_session_id: &mut u64,
    last_saved: &mut f64,
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
) {
    match cmd {
        PlayerCommand::Play { slug, number, url, title, start } => {
            info!(slug, number, url, "reproduciendo");
            if let Ok(mut s) = state.lock() {
                // Guardar el progreso de la sesión activa anterior, si la hay.
                if s.session_id != 0 && s.duration > 0.0 {
                    if let Some(prev_slug) = s.slug.clone() {
                        save_progress(pool, rt, &prev_slug, s.number, s.position, s.duration);
                    }
                }
                // Nueva sesión de reproducción: identidad única y monotónica.
                *next_session_id += 1;
                let id = *next_session_id;
                *session = Some(PlaybackSession {
                    id,
                    slug: slug.clone(),
                    number: *number,
                });
                *awaiting_start = true;
                s.session_id = id;
                s.phase = PlayerPhase::Loading;
                s.title = Some(title.clone());
                s.slug = Some(slug.clone());
                s.number = *number;
                s.error = None;
                s.position = *start;
                s.duration = 0.0;
                s.loaded = false;
                s.playing = false;
                s.buffering = false;
                *last_saved = *start;
            }
            let _ = mpv.command(&["loadfile", url.as_str(), "replace"]);
            if *start > 0.0 {
                let _ = mpv.set_property("time-pos", *start);
            }
            emit_state(state, app);
        }
        PlayerCommand::Pause => {
            let _ = mpv.set_property("pause", true);
        }
        PlayerCommand::Resume => {
            let _ = mpv.set_property("pause", false);
        }
        PlayerCommand::TogglePause => {
            if let Ok(s) = state.lock() {
                let _ = mpv.set_property("pause", s.playing);
            }
        }
        PlayerCommand::Seek(secs) => {
            let _ = mpv.set_property("time-pos", *secs);
        }
        PlayerCommand::SetSpeed(speed) => {
            let s = (*speed).clamp(0.25, 3.0);
            if let Ok(mut st) = state.lock() {
                st.speed = s;
            }
            let _ = mpv.set_property("speed", s);
            emit_state(state, app);
        }
        PlayerCommand::SetVolume(vol) => {
            let v = (*vol).clamp(0, 150);
            if let Ok(mut st) = state.lock() {
                st.volume = v;
            }
            let _ = mpv.set_property("volume", v);
            emit_state(state, app);
        }
        PlayerCommand::ToggleMute => {
            if let Ok(mut st) = state.lock() {
                st.muted = !st.muted;
                let _ = mpv.set_property("mute", st.muted);
            }
            emit_state(state, app);
        }
        PlayerCommand::SetFullscreen(fs) => {
            if let Ok(mut st) = state.lock() {
                st.fullscreen = *fs;
            }
            let _ = mpv.set_property("fullscreen", *fs);
            emit_state(state, app);
        }
        PlayerCommand::SetSubtitles(url) => {
            match url {
                Some(u) => {
                    let _ = mpv.command(&["sub-add", u.as_str(), "select"]);
                }
                None => {
                    let _ = mpv.command(&["set", "sid", "no"]);
                }
            }
        }
        PlayerCommand::Stop => {
            info!("deteniendo reproducción");
            let mut notify = false;
            if let Ok(mut s) = state.lock() {
                if s.session_id != 0 && s.phase != PlayerPhase::Stopping {
                    if s.duration > 0.0 {
                        if let Some(slug) = s.slug.clone() {
                            save_progress(pool, rt, &slug, s.number, s.position, s.duration);
                        }
                    }
                    // Se invalida la sesión: cualquier evento posterior generado
                    // por este stop será ignorado. La fase Stopping se resuelve
                    // con el EndFile(STOP) que mpv emite como confirmación.
                    s.phase = PlayerPhase::Stopping;
                    s.playing = false;
                    s.loaded = false;
                    s.buffering = false;
                    notify = true;
                }
            }
            if notify {
                emit_state(state, app);
            }
            let _ = mpv.command(&["stop"]);
        }
    }
}

/// Guarda el progreso sin bloquear el hilo del reproductor: la persistencia se
/// programa en el runtime asíncrono de Tauri en lugar de ejecutar `block_on`.
fn save_progress(
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
    slug: &str,
    number: i32,
    position: f64,
    duration: f64,
) {
    if duration <= 0.0 {
        return;
    }
    let pool = pool.clone();
    let slug = slug.to_string();
    let fut = async move {
        let anime = crate::infra::repos::AnimeRepo::get_by_slug(&pool, &slug).await.ok().flatten();
        let Some(anime) = anime else { return };
        let episode_id = crate::infra::repos::AnimeRepo::episodes(&pool, anime.id)
            .await
            .ok()
            .and_then(|eps| eps.into_iter().find(|e| e.number == number).map(|e| e.id));
        if let Err(e) = HistoryRepo::upsert(&pool, anime.id, episode_id, position, duration).await {
            warn!(error = %e, "no se pudo guardar progreso");
        }
    };
    let _handle = rt.spawn(fut);
}

fn set_error(state: &Arc<Mutex<PlayerState>>, app: &AppHandle, msg: &str) {
    if let Ok(mut s) = state.lock() {
        s.error = Some(msg.to_string());
    }
    let _ = app.emit("player://error", msg.to_string());
}

fn emit_state(state: &Arc<Mutex<PlayerState>>, app: &AppHandle) {
    if let Ok(s) = state.lock() {
        let _ = app.emit("player://state", s.clone());
    }
}
