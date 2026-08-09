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

#[derive(Debug, Clone, Serialize, Default)]
pub struct PlayerState {
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
    slug: String,
    number: i32,
    position: f64,
    duration: f64,
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
            .name("kakasuma-mpv".into())
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

    // let _ = mpv.command(&["set", "keep-open", "yes"]);
    let _ = mpv.set_property("volume", 100i64);
    let _ = mpv.set_property("speed", 1.0f64);

    info!("libmpv listo");
    emit_state(&state, &app);

    let mut last_saved: f64 = 0.0;
    let mut current: Option<(String, i32)> = None;

    loop {
        // Drenar comandos pendientes.
        loop {
            match rx.try_recv() {
                Ok(cmd) => {
                    apply_command(&cmd, &mut mpv, &state, &app, &mut current, &pool, &rt);
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
        if let Ok(loaded) = mpv.get_property::<bool>("idle-active") {
            let _ = loaded;
        }
        if let (Ok(pos), Ok(dur), Ok(pause), Ok(buffering)) = (
            mpv.get_property::<f64>("time-pos"),
            mpv.get_property::<f64>("duration"),
            mpv.get_property::<bool>("pause"),
            mpv.get_property::<bool>("paused-for-cache"),
        ) {
            let dur = dur.max(0.0);
            let pos = pos.max(0.0);
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.position = pos;
            s.duration = dur;
            s.playing = !pause && dur > 0.0;
            s.loaded = dur > 0.0;
            s.buffering = buffering;
        }
        emit_state(&state, &app);

        // Guardar progreso cada 10 s.
        if let Some((slug, number)) = &current {
            if let Ok(s) = state.lock() {
                if s.duration > 0.0 && s.position - last_saved >= SAVE_INTERVAL_SECS {
                    save_progress(&pool, &rt, slug, *number, s.position, s.duration);
                    let _ = app.emit(
                        "player://progress",
                        ProgressEvent {
                            slug: slug.clone(),
                            number: *number,
                            position: s.position,
                            duration: s.duration,
                        },
                    );
                    last_saved = s.position;
                }
            }
        }

        // Eventos de mpv.
        while let Some(ev) = mpv.wait_event(0.0) {
            match ev {
                mpv::Event::StartFile => {
                    debug!("mpv: archivo iniciado");
                    if let Some((slug, number)) = &current {
                        if let Ok(mut s) = state.lock() {
                            s.loaded = true;
                            s.playing = true;
                            s.slug = Some(slug.clone());
                            s.number = *number;
                        }
                        emit_state(&state, &app);
                    }
                }
                mpv::Event::FileLoaded => {
                    if let Some((_slug, _number)) = &current {
                        // Aplicar posición de reanudación.
                        if let Ok(s) = state.lock() {
                            if s.position > 5.0 {
                                let _ = mpv.set_property("time-pos", s.position);
                            }
                        }
                    }
                }
                mpv::Event::EndFile(reason) => {
                    debug!("mpv: fin de archivo ({reason:?})");

                    if let Some((slug, number)) = &current {
                        if let Ok(mut s) = state.lock() {
                            save_progress(&pool, &rt, slug, *number, s.position, s.duration);

                            let _ = app.emit(
                                "player://end",
                                ProgressEvent {
                                    slug: slug.clone(),
                                    number: *number,
                                    position: s.position,
                                    duration: s.duration,
                                },
                            );

                            s.playing = false;
                            s.loaded = false;
                            s.position = 0.0;
                            s.duration = 0.0;
                        }

                        current.take();

                        // Oculta la ventana de reproducción cuando termina el video
                        let _ = mpv.command(&["stop"]);

                        emit_state(&state, &app);
                        info!("cerrando reproducción y limpiando estado");
                    }
                }
                mpv::Event::Idle => {
                    if let Ok(mut s) = state.lock() {
                        s.playing = false;
                    }
                    emit_state(&state, &app);
                }
                _ => {}
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

fn apply_command(
    cmd: &PlayerCommand,
    mpv: &mut Box<mpv::MpvHandler>,
    state: &Arc<Mutex<PlayerState>>,
    app: &AppHandle,
    current: &mut Option<(String, i32)>,
    pool: &SqlitePool,
    rt: &tauri::async_runtime::RuntimeHandle,
) {
    match cmd {
        PlayerCommand::Play { slug, number, url, title, start } => {
            info!(slug, number, url, "reproduciendo");
            if let Ok(mut s) = state.lock() {
                if let (Some(prev_slug), Some(prev_number)) = (s.slug.clone(), Some(s.number)) {
                    if s.duration > 0.0 {
                        save_progress(pool, rt, &prev_slug, prev_number, s.position, s.duration);
                    }
                }
                s.title = Some(title.clone());
                s.slug = Some(slug.clone());
                s.number = *number;
                s.error = None;
                s.position = *start;
                s.loaded = false;
                s.playing = false;
                s.buffering = false;
            }
            *current = Some((slug.clone(), *number));
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
            if let Ok(mut s) = state.lock() {
                if let (Some(slug), true) = (s.slug.clone(), s.duration > 0.0) {
                    save_progress(pool, rt, &slug, s.number, s.position, s.duration);
                }
                s.playing = false;
                s.loaded = false;
            }
            let _ = mpv.command(&["stop"]);
            current.take();
            emit_state(state, app);
        }
    }
}

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
    rt.block_on(fut);
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
