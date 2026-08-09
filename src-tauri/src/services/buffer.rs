use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tracing::{debug, info, warn};

use crate::domain::VideoSource;
use crate::error::AppResult;
use crate::infra::hls::{key_uris_match, wait_if_paused, HlsClient, HlsFetch, TokenBucket};
use crate::infra::http::HttpClient;
use crate::infra::repos::AnimeRepo;
use crate::infra::segcache::SegmentCache;
use crate::infra::segserver::SegmentServer;
use crate::services::{AnimeService, PlayerState};
use crate::settings::{BufferConfig, SettingsService};

// ---------- constantes ----------

const LOOP_INTERVAL: Duration = Duration::from_secs(1);
const PAUSE_POLL: Duration = Duration::from_millis(250);
const CPU_SAMPLE_EVERY: Duration = Duration::from_secs(2);
const CONSECUTIVE_ERROR_THRESHOLD: u32 = 3;
const NETWORK_BACKOFF: Duration = Duration::from_secs(10);
const CONSUMED_PROGRESS: f64 = 0.9;

// bits de condiciones de pausa
const PAUSE_BUFFERING: u64 = 1 << 0;
const PAUSE_NOT_PLAYING: u64 = 1 << 1;
const PAUSE_CPU: u64 = 1 << 2;
const PAUSE_DISK: u64 = 1 << 3;
const PAUSE_NETWORK: u64 = 1 << 4;
const PAUSE_MANUAL: u64 = 1 << 5;

// ---------- estructuras observables ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BufferedEpisode {
    pub slug: String,
    pub number: i32,
    pub percent_done: u32,
    pub segments_total: u32,
    pub segments_done: u32,
    pub bytes: u64,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BufferStatus {
    pub enabled: bool,
    pub paused: bool,
    pub pause_reasons: Vec<String>,
    pub cache_bytes: u64,
    pub cache_limit_bytes: u64,
    pub current_episode: Option<BufferedEpisode>,
    pub queue: Vec<BufferedEpisode>,
}

// ---------- manifest interno ----------

#[derive(Debug, Serialize, Deserialize)]
struct ManifestSegment {
    uri: String,
    file: String,
    duration: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    slug: String,
    number: i32,
    fetched_at: i64,
    target_duration: f64,
    key_uri: Option<String>,
    segments: Vec<ManifestSegment>,
}

#[derive(Debug)]
enum JobOutcome {
    Done,
    Failed(String),
    Unsupported(&'static str),
    Cancelled,
}

// ---------- comandos de control ----------

enum BufferCommand {
    ConfigChanged,
    ClearCache,
    Shutdown,
}

// ---------- servicio ----------

/// Servicio de buffer inteligente en segundo plano.
///
/// Corre en su propio hilo (`kakazuma-buffer`) ejecutando un bucle asíncrono
/// que vigila el estado del reproductor, genera la cola de episodios futuros y
/// descarga un porcentaje de cada uno a disco. Nunca bloquea el hilo de libmpv
/// ni el IPC; solo lee el estado compartido del reproductor.
pub struct BufferService {
    tx: std::sync::mpsc::Sender<BufferCommand>,
    cache: Arc<SegmentCache>,
    status: Arc<Mutex<BufferStatus>>,
    pause: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    settings: Arc<SettingsService>,
    http: HttpClient,
    /// Base URL del servidor local de segmentos (ej. `http://127.0.0.1:PORT`).
    base_url: Option<String>,
    /// El servidor local; se apaga junto con el servicio.
    segserver: Option<Arc<SegmentServer>>,
}

impl BufferService {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        app: AppHandle,
        anime: Arc<AnimeService>,
        pool: SqlitePool,
        http: HttpClient,
        cache: Arc<SegmentCache>,
        settings: Arc<SettingsService>,
        player_state: Arc<Mutex<PlayerState>>,
        rt: tauri::async_runtime::RuntimeHandle,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<BufferCommand>();
        let status = Arc::new(Mutex::new(BufferStatus::default()));
        let pause = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        let (segserver, base_url) = match SegmentServer::start(cache.dir().to_path_buf()) {
            Ok((server, port)) => (Some(server), Some(format!("http://127.0.0.1:{port}"))),
            Err(e) => {
                warn!(error = %e, "buffer sin servidor local: reproducción remota");
                (None, None)
            }
        };

        let thread_status = status.clone();
        let thread_pause = pause.clone();
        let thread_shutdown = shutdown.clone();
        let thread_http = http.clone();
        let thread_cache = cache.clone();
        let thread_settings = settings.clone();
        std::thread::Builder::new()
            .name("kakazuma-buffer".into())
            .spawn(move || {
                rt.block_on(buffer_loop(
                    app,
                    anime,
                    pool,
                    thread_http,
                    thread_cache,
                    thread_settings,
                    player_state,
                    rx,
                    thread_status,
                    thread_pause,
                    thread_shutdown,
                ));
            })
            .map_err(|e| warn!(error = %e, "no se pudo crear el hilo del buffer"))
            .ok();

        Self {
            tx,
            cache,
            status,
            pause,
            shutdown,
            settings,
            http,
            base_url,
            segserver,
        }
    }

    pub fn cache(&self) -> &Arc<SegmentCache> {
        &self.cache
    }

    pub fn status(&self) -> BufferStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// URL final a reproducir: playlist híbrida local-first si hay buffer
    /// válido para el episodio; si no, la URL remota original. Nunca falla.
    pub async fn resolve_playback_url(
        &self,
        source: &VideoSource,
        slug: &str,
        number: i32,
    ) -> String {
        let cfg = self.settings.config();
        if !cfg.smart_buffer_enabled || cfg.buffer_episode_count == 0 {
            return source.url.clone();
        }

        let Some(manifest) = self.read_manifest(slug, number) else {
            return source.url.clone();
        };
        if manifest.segments.is_empty() {
            return source.url.clone();
        }

        // Re-resolver el nivel con tokens frescos desde el maestro actual.
        let hls = HlsClient::new(self.http.clone());
        let fetch = match hls.resolve_level(&source.url).await {
            Ok(f) => f,
            Err(e) => {
                debug!(slug, number, error = %e, "sin buffer: no se pudo revalidar HLS");
                return source.url.clone();
            }
        };
        let HlsFetch::Ready { level } = fetch else {
            return source.url.clone();
        };

        // Los segmentos cacheados deben coincidir con el prefijo fresco.
        if level.segments.len() < manifest.segments.len() {
            return source.url.clone();
        }
        for (cached, fresh) in manifest.segments.iter().zip(level.segments.iter()) {
            if cached.uri != fresh.uri {
                return source.url.clone();
            }
        }

        // Coherencia de cifrado entre el buffer y la playlist fresca.
        let fresh_key = crate::infra::hls::first_key_uri(&level);
        match (&fresh_key, &manifest.key_uri) {
            (Some(fk), Some(ck)) => {
                if !key_uris_match(fk, ck) {
                    return source.url.clone();
                }
            }
            (None, None) => {}
            _ => return source.url.clone(),
        }

        let local: Vec<(PathBuf, f64)> = manifest
            .segments
            .iter()
            .map(|s| {
                let name = Path::new(&s.file)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| s.file.clone());
                (PathBuf::from(name), s.duration)
            })
            .collect();
        let tail = &level.segments[manifest.segments.len()..];
        let tail_key = level
            .keys
            .get(manifest.segments.len())
            .and_then(|k| k.as_ref());
        let playlist = HlsClient::build_hybrid_playlist(
            &local,
            tail,
            tail_key,
            level.target_duration,
        );

        let dir = self.cache.episode_dir(slug, number);
        if std::fs::create_dir_all(&dir).is_err() {
            return source.url.clone();
        }
        let out = dir.join("playback.m3u8");
        if std::fs::write(&out, playlist).is_err() {
            return source.url.clone();
        }

        let Some(base) = &self.base_url else {
            info!(slug, number, "buffer: sin servidor local, playlist remota");
            return source.url.clone();
        };
        let hash = self
            .cache
            .slug_dir(slug)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        info!(slug, number, cached = manifest.segments.len(), "reproducción con buffer local-first");
        format!("{base}/buffer/{hash}/{number}/playback.m3u8")
    }

    pub fn config_changed(&self) {
        let _ = self.tx.send(BufferCommand::ConfigChanged);
    }

    pub fn clear_cache(&self) {
        let _ = self.tx.send(BufferCommand::ClearCache);
    }

    pub fn set_manual_pause(&self, paused: bool) {
        set_pause_bit(&self.pause, PAUSE_MANUAL, paused);
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(server) = &self.segserver {
            server.shutdown();
        }
        let _ = self.tx.send(BufferCommand::Shutdown);
    }

    fn read_manifest(&self, slug: &str, number: i32) -> Option<Manifest> {
        let path = self.cache.episode_dir(slug, number).join("manifest.json");
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

// ---------- bucle principal ----------

#[allow(clippy::too_many_arguments)]
async fn buffer_loop(
    app: AppHandle,
    anime: Arc<AnimeService>,
    pool: SqlitePool,
    http: HttpClient,
    cache: Arc<SegmentCache>,
    settings: Arc<SettingsService>,
    player_state: Arc<Mutex<PlayerState>>,
    rx: std::sync::mpsc::Receiver<BufferCommand>,
    status: Arc<Mutex<BufferStatus>>,
    pause: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
) {
    let cfg = settings.config();
    cache.set_limit(cfg.buffer_cache_limit_mb);

    let throttle = Arc::new(tokio::sync::Mutex::new(TokenBucket::new(
        cfg.buffer_bandwidth_limit_mbps,
    )));
    let mut set = tokio::task::JoinSet::new();

    // estado interno
    let mut anchor: Option<(String, i32)> = None;
    let mut anchor_max_episode: i32 = 0;
    let mut queued_for_anchor = false;
    let mut pending: VecDeque<(String, i32)> = VecDeque::new();
    let mut consecutive_errors: u32 = 0;
    let gen = Arc::new(AtomicU64::new(0));
    let mut cpu_sys: Option<sysinfo::System> = None;
    let mut last_cpu = Instant::now() - CPU_SAMPLE_EVERY;

    info!("buffer inteligente iniciado");
    emit_status(&app, &status, &cfg, &cache, &pause, &anchor);

    loop {
        // comandos de control
        loop {
            match rx.try_recv() {
                Ok(BufferCommand::ConfigChanged) => {
                    let new_cfg = settings.config();
                    cache.set_limit(new_cfg.buffer_cache_limit_mb);
                    {
                        let mut b = throttle.lock().await;
                        b.set_rate(new_cfg.buffer_bandwidth_limit_mbps);
                    }
                    gen.fetch_add(1, Ordering::Relaxed);
                    set.abort_all();
                    pending.clear();
                    queued_for_anchor = false;
                    consecutive_errors = 0;
                    debug!("config de buffer aplicada");
                }
                Ok(BufferCommand::ClearCache) => {
                    gen.fetch_add(1, Ordering::Relaxed);
                    set.abort_all();
                    pending.clear();
                    queued_for_anchor = false;
                    let _ = cache.clear();
                    debug!("caché de buffer limpiada");
                }
                Ok(BufferCommand::Shutdown) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    set.abort_all();
                    info!("buffer detenido");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        if shutdown.load(Ordering::Relaxed) {
            set.abort_all();
            info!("buffer detenido");
            return;
        }

        // estado del reproductor (solo lectura)
        let ps = player_state.lock().map(|s| s.clone()).unwrap_or_default();
        let cfg = settings.config();
        let enabled = cfg.smart_buffer_enabled && cfg.buffer_episode_count > 0;

        // condiciones de pausa: buffering y pausa manual
        set_pause_bit(&pause, PAUSE_BUFFERING, ps.buffering);
        set_pause_bit(&pause, PAUSE_NOT_PLAYING, ps.loaded && !ps.playing);

        // CPU
        if last_cpu.elapsed() >= CPU_SAMPLE_EVERY {
            last_cpu = Instant::now();
            let sys = cpu_sys.get_or_insert_with(sysinfo::System::new);
            sys.refresh_cpu_usage();
            let usage = sys.global_cpu_usage();
            set_pause_bit(&pause, PAUSE_CPU, usage > cfg.buffer_cpu_threshold_percent as f32);
        }

        // disco
        if cache.over_limit() {
            set_pause_bit(&pause, PAUSE_DISK, true);
            let _ = cache.enforce_limit().await;
        } else {
            set_pause_bit(&pause, PAUSE_DISK, false);
        }

        // red (backoff tras errores consecutivos)
        if consecutive_errors >= CONSECUTIVE_ERROR_THRESHOLD {
            set_pause_bit(&pause, PAUSE_NETWORK, true);
        } else {
            set_pause_bit(&pause, PAUSE_NETWORK, false);
        }

        // gestión del episodio en reproducción
        if ps.loaded && ps.duration > 0.0 && ps.playing {
            let current = (ps.slug.clone().unwrap_or_default(), ps.number);
            let progress = (ps.position / ps.duration).clamp(0.0, 1.0);

            if anchor.as_ref() != Some(&current) {
                gen.fetch_add(1, Ordering::Relaxed);
                set.abort_all();
                pending.clear();
                queued_for_anchor = false;
                consecutive_errors = 0;
                if let Ok(mut s) = status.lock() {
                    s.queue.clear();
                }
                anchor = Some(current.clone());
                anchor_max_episode = max_episode(&pool, &current.0).await.unwrap_or(0);
                info!(slug = %current.0, number = current.1, "ancla de buffer actualizada");
            }

            // la caché del episodio que se está consumiendo ya no sirve
            if progress >= CONSUMED_PROGRESS {
                let _ = cache.remove_episode(&current.0, current.1);
            }

            // trigger: encolar episodios futuros una sola vez por ancla
            if enabled && !queued_for_anchor && progress >= cfg.buffer_trigger_percent as f64 / 100.0 {
                let start = current.1 + 1;
                let end = (current.1 + cfg.buffer_episode_count as i32).min(anchor_max_episode);
                if start <= end {
                    for n in start..=end {
                        pending.push_back((current.0.clone(), n));
                        upsert_entry(&status, &gen, gen.load(Ordering::Relaxed), &current.0, n, "queued");
                    }
                    queued_for_anchor = true;
                    info!(slug = %current.0, start, end, "cola de buffer generada");
                }
            }
        } else if !ps.loaded {
            gen.fetch_add(1, Ordering::Relaxed);
            set.abort_all();
            pending.clear();
            queued_for_anchor = false;
            anchor = None;
            if let Ok(mut s) = status.lock() {
                s.queue.clear();
            }
        }

        // arrancar un job si hay cola, no hay job activo y no hay pausa
        if enabled && !pending.is_empty() && set.is_empty() {
            let (slug, number) = pending.pop_front().unwrap();
            let gstart = gen.load(Ordering::Relaxed);
            upsert_entry(&status, &gen, gstart, &slug, number, "downloading");

            let job_app = app.clone();
            let job_anime = anime.clone();
            let job_http = http.clone();
            let job_cache = cache.clone();
            let job_throttle = throttle.clone();
            let job_pause = pause.clone();
            let job_shutdown = shutdown.clone();
            let job_status = status.clone();
            let job_gen = gen.clone();
            let job_cfg = cfg.clone();
            set.spawn(async move {
                download_job(
                    job_app,
                    job_anime,
                    job_http,
                    job_cache,
                    job_throttle,
                    job_pause,
                    job_shutdown,
                    job_status,
                    job_gen,
                    gstart,
                    slug,
                    number,
                    job_cfg,
                )
                .await
            });
        }

        // recolectar resultados (no bloqueante)
        while let Some(res) = set.try_join_next() {
            match res {
                Ok(JobOutcome::Done) => consecutive_errors = 0,
                Ok(JobOutcome::Failed(e)) => {
                    consecutive_errors += 1;
                    warn!(error = %e, "job de buffer falló");
                }
                Ok(JobOutcome::Unsupported(reason)) => {
                    warn!(reason, "HLS no soportado para buffer");
                }
                Ok(JobOutcome::Cancelled) | Err(_) => {}
            }
        }

        // esperar el backoff de red dentro del propio bucle
        if pause.load(Ordering::Relaxed) & PAUSE_NETWORK != 0 {
            tokio::time::sleep(NETWORK_BACKOFF).await;
            consecutive_errors = 0;
        }

        emit_status(&app, &status, &cfg, &cache, &pause, &anchor);
        tokio::time::sleep(LOOP_INTERVAL).await;
    }
}

// ---------- job de descarga ----------

#[allow(clippy::too_many_arguments)]
async fn download_job(
    app: AppHandle,
    anime: Arc<AnimeService>,
    http: HttpClient,
    cache: Arc<SegmentCache>,
    throttle: Arc<tokio::sync::Mutex<TokenBucket>>,
    pause: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    status: Arc<Mutex<BufferStatus>>,
    gen: Arc<AtomicU64>,
    gen_at_start: u64,
    slug: String,
    number: i32,
    cfg: BufferConfig,
) -> JobOutcome {
    {
        let mut b = throttle.lock().await;
        b.set_rate(cfg.buffer_bandwidth_limit_mbps);
    }

    // 1. resolver el video del episodio futuro
    let source = match anime.resolve_video(&slug, number).await {
        Ok(s) => s,
        Err(e) => {
            set_entry(&status, &gen, gen_at_start, &slug, number, "error", Some(e.to_string()));
            return JobOutcome::Failed(e.to_string());
        }
    };

    // 2. resolver la playlist de nivel
    let hls = HlsClient::new(http);
    let fetch = match hls.resolve_level(&source.url).await {
        Ok(f) => f,
        Err(e) => {
            set_entry(&status, &gen, gen_at_start, &slug, number, "error", Some(e.to_string()));
            return JobOutcome::Failed(e.to_string());
        }
    };
    let level = match fetch {
        HlsFetch::Ready { level } => level,
        HlsFetch::Unsupported(reason) => {
            set_entry(&status, &gen, gen_at_start, &slug, number, "unsupported", Some(reason.to_string()));
            return JobOutcome::Unsupported(reason);
        }
    };

    let n = HlsClient::segment_range(level.segments.len(), cfg.buffer_percentage);
    if n == 0 {
        set_entry(&status, &gen, gen_at_start, &slug, number, "done", None);
        return JobOutcome::Done;
    }

    let dir = cache.episode_dir(&slug, number);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        set_entry(&status, &gen, gen_at_start, &slug, number, "error", Some(format!("crear dir: {e}")));
        return JobOutcome::Failed(format!("crear dir: {e}"));
    }

    // 3. descargar segmentos (secuencial, con throttle y pausas)
    let mut bytes = 0u64;
    let total = n as u32;
    for (i, seg) in level.segments.iter().take(n).enumerate() {
        wait_if_paused(&pause, &shutdown).await;
        if shutdown.load(Ordering::Relaxed) {
            return JobOutcome::Cancelled;
        }
        if gen.load(Ordering::Relaxed) != gen_at_start {
            return JobOutcome::Cancelled;
        }

        let out_path = dir.join(format!("seg_{i:04}.ts"));
        match hls.download_segment(&seg.uri, &out_path, &throttle, &pause, &shutdown).await {
            Ok(len) => {
                bytes += len;
                cache.add_bytes(len);
                let _ = cache.enforce_limit().await;
                let done = i as u32 + 1;
                update_entry(&status, &gen, gen_at_start, &slug, number, |e| {
                    e.state = "downloading".to_string();
                    e.segments_total = total;
                    e.segments_done = done;
                    e.percent_done = (done as u64 * 100 / total.max(1) as u64) as u32;
                    e.bytes = bytes;
                });
            }
            Err(e) => {
                set_entry(&status, &gen, gen_at_start, &slug, number, "error", Some(e.to_string()));
                return JobOutcome::Failed(e.to_string());
            }
        }
    }

    // 4. escribir el manifest
    let manifest = Manifest {
        slug: slug.clone(),
        number,
        fetched_at: chrono::Utc::now().timestamp(),
        target_duration: level.target_duration,
        key_uri: crate::infra::hls::first_key_uri(&level),
        segments: level
            .segments
            .iter()
            .take(n)
            .enumerate()
            .map(|(i, s)| ManifestSegment {
                uri: s.uri.clone(),
                file: dir.join(format!("seg_{i:04}.ts")).display().to_string(),
                duration: s.duration,
            })
            .collect(),
    };
    if let Ok(raw) = serde_json::to_vec(&manifest) {
        let tmp = dir.join("manifest.json.tmp");
        let _ = std::fs::write(&tmp, raw);
        let _ = std::fs::rename(&tmp, dir.join("manifest.json"));
    }

    set_entry(&status, &gen, gen_at_start, &slug, number, "done", None);
    info!(slug, number, segments = n, bytes, "buffer del episodio completado");
    let _ = app;
    JobOutcome::Done
}

// ---------- helpers de estado ----------

fn pause_reasons(mask: u64) -> Vec<String> {
    let mut out = Vec::new();
    if mask & PAUSE_BUFFERING != 0 {
        out.push("buffering".into());
    }
    if mask & PAUSE_NOT_PLAYING != 0 {
        out.push("not_playing".into());
    }
    if mask & PAUSE_CPU != 0 {
        out.push("cpu".into());
    }
    if mask & PAUSE_DISK != 0 {
        out.push("disk".into());
    }
    if mask & PAUSE_NETWORK != 0 {
        out.push("network".into());
    }
    if mask & PAUSE_MANUAL != 0 {
        out.push("manual".into());
    }
    out
}

fn set_pause_bit(pause: &AtomicU64, bit: u64, on: bool) {
    let mut v = pause.load(Ordering::Relaxed);
    loop {
        let nv = if on { v | bit } else { v & !bit };
        match pause.compare_exchange_weak(v, nv, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(cur) => v = cur,
        }
    }
}

fn upsert_entry(
    status: &Arc<Mutex<BufferStatus>>,
    gen: &Arc<AtomicU64>,
    gen_at_start: u64,
    slug: &str,
    number: i32,
    state: &str,
) {
    if gen.load(Ordering::Relaxed) != gen_at_start {
        return;
    }
    if let Ok(mut s) = status.lock() {
        match s.queue.iter_mut().find(|e| e.slug == slug && e.number == number) {
            Some(e) => e.state = state.to_string(),
            None => s.queue.push(BufferedEpisode {
                slug: slug.to_string(),
                number,
                state: state.to_string(),
                ..Default::default()
            }),
        }
    }
}

fn set_entry(
    status: &Arc<Mutex<BufferStatus>>,
    gen: &Arc<AtomicU64>,
    gen_at_start: u64,
    slug: &str,
    number: i32,
    state: &str,
    error: Option<String>,
) {
    update_entry(status, gen, gen_at_start, slug, number, |e| {
        e.state = state.to_string();
        e.error = error;
    });
}

fn update_entry(
    status: &Arc<Mutex<BufferStatus>>,
    gen: &Arc<AtomicU64>,
    gen_at_start: u64,
    slug: &str,
    number: i32,
    f: impl FnOnce(&mut BufferedEpisode),
) {
    if gen.load(Ordering::Relaxed) != gen_at_start {
        return;
    }
    if let Ok(mut s) = status.lock() {
        match s.queue.iter_mut().find(|e| e.slug == slug && e.number == number) {
            Some(e) => f(e),
            None => {
                s.queue.push(BufferedEpisode {
                    slug: slug.to_string(),
                    number,
                    ..Default::default()
                });
                let last = s.queue.last_mut().unwrap();
                f(last);
            }
        }
    }
}

fn emit_status(
    app: &AppHandle,
    status: &Arc<Mutex<BufferStatus>>,
    cfg: &BufferConfig,
    cache: &SegmentCache,
    pause: &Arc<AtomicU64>,
    anchor: &Option<(String, i32)>,
) {
    let mask = pause.load(Ordering::Relaxed);
    let entries = status.lock().map(|s| s.queue.clone()).unwrap_or_default();
    let current_episode = anchor.as_ref().map(|(slug, number)| BufferedEpisode {
        slug: slug.clone(),
        number: *number,
        ..Default::default()
    });

    let new_status = BufferStatus {
        enabled: cfg.smart_buffer_enabled && cfg.buffer_episode_count > 0,
        paused: mask != 0,
        pause_reasons: pause_reasons(mask),
        cache_bytes: cache.bytes(),
        cache_limit_bytes: cache.limit(),
        current_episode,
        queue: entries,
    };

    let changed = match status.lock() {
        Ok(mut s) => {
            let changed = *s != new_status;
            *s = new_status.clone();
            changed
        }
        Err(_) => false,
    };
    if changed {
        let _ = app.emit("buffer://status", &new_status);
    }
}

// ---------- helpers de datos ----------

async fn max_episode(pool: &SqlitePool, slug: &str) -> AppResult<i32> {
    let anime = AnimeRepo::get_by_slug(pool, slug).await?;
    let Some(anime) = anime else {
        return Ok(0);
    };
    let episodes = AnimeRepo::episodes(pool, anime.id).await?;
    Ok(episodes.iter().map(|e| e.number).max().unwrap_or(0))
}
