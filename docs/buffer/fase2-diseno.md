# Fase 2 — Diseño detallado

> Sobre la arquitectura existente. Sin arquitectura paralela.

---

## 2.1 Módulos nuevos

```
src-tauri/src/
  settings.rs              SettingsService: config persistente + acceso tipado
  services/buffer.rs       BufferService: worker async + cola + jobs + throttling + status
  infra/hls.rs             HlsClient: parser m3u8 (maestro/nivel), descarga de segmentos, builder de playlist híbrida
  infra/segcache.rs        SegmentCache: directorio, índice en memoria, evicción LRU por bytes

src/components/SettingsView.tsx    UI de ajustes (nuevo tab "Ajustes")
src/stores/useAppStore.ts          estado + listeners del buffer (extender)
src/lib/api.ts / types.ts          wrappers IPC + tipos (extender)
src-tauri/migrations/0002_settings.sql
```

Integración con lo existente:
- `AppState` gana dos campos: `settings: SettingsService` y `buffer: BufferService`.
- `commands.rs` registra nuevos comandos (`buffer_*`, `settings_*`).
- `PlayerService` solo se toca para (a) añadir `buffering: bool` al estado (poll de `paused-for-cache`) y (b) setear opciones de caché de mpv. Su bucle y contrato no cambian.
- El worker del buffer lee el `Arc<Mutex<PlayerState>>` compartido (solo lectura) y reutiliza `AnimeService::resolve_video`.

## 2.2 Estructuras Rust

```rust
// ---- settings.rs -------------------------------------------------------

/// Configuración persistente del Smart Buffer (tabla `settings`, key="smart_buffer").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferConfig {
    pub smart_buffer_enabled: bool,          // default true
    pub buffer_episode_count: u32,           // 0..=5   (0 = desactivado), default 1
    pub buffer_percentage: u32,              // 10|20|30|50 (nunca 100), default 20
    pub buffer_cache_limit_mb: u64,          // 500|1000|2000|5000, default 1000
    pub buffer_bandwidth_limit_mbps: u64,    // 1|2|5|10, default 5
    pub buffer_trigger_percent: u32,         // 60..=95, default 80 (inicio del precache)
    pub buffer_cpu_threshold_percent: u32,   // 0..=100, default 70
}

impl Default for BufferConfig { ... }  // conservador

pub struct SettingsService {
    inner: Arc<RwLock<BufferConfig>>,   // copia en memoria (tokio RwLock)
    pool: SqlitePool,
}
impl SettingsService {
    pub async fn load(&self) -> AppResult<()>;                    // desde DB (o defaults)
    pub fn config(&self) -> BufferConfig;                          // clon rápido
    pub async fn set_config(&self, cfg: BufferConfig) -> AppResult<BufferConfig>; // valida + persiste
}
```

```rust
// ---- infra/segcache.rs ------------------------------------------------

pub struct SegmentCache {
    dir: PathBuf,                       // {app_data}/buffer
    bytes: AtomicU64,                   // total de bytes en caché
    limit_bytes: AtomicU64,             // buffer_cache_limit_mb * 1_000_000
}
impl SegmentCache {
    pub fn new(base: &Path) -> AppResult<Self>;      // crea dir, escanea tamaño inicial
    pub fn episode_dir(&self, slug: &str, number: i32) -> PathBuf;  // {dir}/{sha256(slug)}/{number}
    pub fn add_bytes(&self, n: u64);                 // contador
    pub fn bytes(&self) -> u64;
    pub fn limit(&self) -> u64;
    pub async fn enforce_limit(&self) -> AppResult<()>;  // purge LRU (mtime) hasta 80% límite
    pub fn clear(&self) -> AppResult<()>;            // borrado total
}
```

```rust
// ---- infra/hls.rs -----------------------------------------------------

pub struct HlsSegment { pub uri: String, pub duration: f64, pub byte_range: Option<(u64, u64)> }
pub struct HlsKey     { pub method: String, pub uri: Option<String>, pub iv: Option<String> }

pub struct LevelPlaylist {
    pub segments: Vec<HlsSegment>,
    pub keys: Vec<Option<HlsKey>>,     // clave por segmento (índice paralelo)
    pub target_duration: f64,
    pub end_list: bool,
    pub base_url: String,              // base para resolver URIs relativas
}

pub enum HlsFetch {
    Unsupported(&'static str),          // live / byterange → abortar
    Ready { level: LevelPlaylist, variant_url: String },
}

pub struct HlsClient { http: HttpClient }
impl HlsClient {
    pub async fn resolve_level(&self, master_url: &str) -> AppResult<HlsFetch>;
    //  1. GET maestro. Si contiene #EXT-X-STREAM-INF → elegir 1.ª variante estable
    //     (preferir la que tiene RESOLUTION/bitrate explícito), GET al nivel.
    //  2. Parsear nivel. Si !end_list → Unsupported("live"). Si byte_range → Unsupported.
    //  3. Resolver URIs relativas contra base_url.
    pub fn segment_range(&self, level: &LevelPlaylist, percent: u32) -> usize;  // nº segmentos a cachear
    pub fn build_hybrid_playlist(
        local: &[(String /*abs path*/, f64)],
        remote_tail: &[HlsSegment],
        keys_local: Vec<Option<PathBuf>>,
        target_duration: f64,
    ) -> String;  // m3u8 con EXT-X-KEY local, segmentos locales (rutas abs) + tail remoto
}
```

```rust
// ---- services/buffer.rs -----------------------------------------------

#[derive(Clone, Serialize, Default)]
pub struct BufferedEpisode {
    pub slug: String,
    pub number: i32,
    pub percent_done: u32,     // progreso real del buffer
    pub segments_total: u32,
    pub segments_done: u32,
    pub bytes: u64,
    pub state: String,         // queued|downloading|done|paused|error|cancelled|unsupported
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Default)]
pub struct BufferStatus {
    pub enabled: bool,
    pub running: bool,
    pub paused: bool,
    pub pause_reasons: Vec<String>,
    pub cache_bytes: u64,
    pub cache_limit_bytes: u64,
    pub queue: Vec<BufferedEpisode>,     // pendientes + activo
    pub current: Option<BufferedEpisode>, // episodio en reproducción (origen del trigger)
}

struct PauseReasons { buffering: bool, not_playing: bool, cpu: bool, disk: bool, network: bool, manual: bool }

pub struct BufferService {
    app: AppHandle,
    anime: AnimeService,                    // para resolve_video de episodios futuros
    pool: SqlitePool,
    cache: Arc<SegmentCache>,
    hls: HlsClient,
    player_state: Arc<Mutex<PlayerState>>,  // solo lectura
    settings: Arc<SettingsService>,
    tx: std::sync::mpsc::Sender<BufferCommand>,
    rx: std::sync::mpsc::Receiver<BufferCommand>,   // canal de control
    status: Arc<Mutex<BufferStatus>>,
    pause: Arc<AtomicU64>,                  // bitmask de razones de pausa
    shutdown: Arc<AtomicBool>,
    rt: tauri::async_runtime::RuntimeHandle,
}

enum BufferCommand {
    ConfigChanged,
    ClearCache,
    Shutdown,
}
```

**Diseño del worker**: `BufferService::spawn(...)` crea un hilo ligero (`std::thread`, nombre `kakasuma-buffer`) cuyo único trabajo es ejecutar `rt.block_on(buffer_loop(...))`. De esta forma el bucle vive fuera del runtime de Tauri sin bloquear IPC y puede usar Tokio internamente (tareas, sleep, timeouts).

`buffer_loop` (bucle principal, cadencia 1 s):
1. Procesa comandos del canal (config changed, clear, shutdown).
2. Lee `PlayerState` (posición, playing, buffering).
3. Detecta episodio nuevo / trigger de progreso → genera cola de próximos episodios.
4. Consume la cola: inicia **un** `download_job` (JoinSet) si no hay uno activo.
5. Actualiza condiciones de pausa (buffering, playing, CPU, disco, red) → bitmask.
6. Emite `buffer://status` (con throttle: solo si cambia algo).

```rust
async fn download_job(app, anime, hls, cache, settings, pause, status, slug, number) {
    // 1. resolve_video(slug, number)  →  URL HLS maestro
    // 2. hls.resolve_level(url)       →  Unsupported → estado "unsupported", fin
    // 3. n = segment_range(level, percentage)
    // 4. para i in 0..n:
    //      - esperar mientras pause != 0 (poll 250 ms, con shutdown check)
    //      - descargar segmento con token bucket (throttle por bandwidth)
    //      - guardar como {episode_dir}/seg_{i}.ts (tmp + rename)
    //      - cache.add_bytes(len); enforce_limit()
    // 5. escribir manifest.json { level_url, base_url, fetched_at,
    //        segments: [{uri, file}] , keys: [..] }
}
```

**Throttle de ancho de banda** (token bucket, compartido por `Arc<tokio::sync::Mutex<TokenBucket>>`):
- `capacity = bandwidth_mbps * 1e6 / 8 * 1s` tokens; recarga continua.
- Cada descarga de segmento usa `resp.bytes_stream()` (reqwest feature `stream`), lee en chunks de 64 KiB consumiendo tokens; si no hay tokens, espera. Así se puede pausar a mitad de segmento (check de `pause` por chunk).

**Detección de condiciones** (requisitos del enunciado):

| Condición | Mecanismo | Costo |
|---|---|---|
| mpv en buffering | `PlayerState.buffering` (nuevo campo, poll de `paused-for-cache` en el hilo mpv) | 1 read/1s |
| pausa manual / no reproducción | `PlayerState.playing == false` | 1 read/1s |
| CPU alta | crate `sysinfo`: `global_cpu_usage()` muestreado cada 2 s | bajo |
| disco lleno | `SegmentCache.bytes >= limit` → pausa + purge LRU | contador atómico |
| red saturada | throttle propio (nunca supera límite) + `download_job` con reintentos/backoff; si 3 errores seguidos → backoff 10 s y estado "network" | interno |

**Playback URL (punto único de integración)**:

```rust
impl BufferService {
    pub async fn resolve_playback_url(&self, source: &VideoSource, slug: &str, number: i32) -> String {
        let cfg = settings.config();
        if !cfg.smart_buffer_enabled || cfg.buffer_episode_count == 0 { return source.url.clone(); }
        // lee manifest del episodio
        let Ok(Some(manifest)) = manifest_for(slug, number) else { return source.url.clone(); };
        // 1. re-resuelve nivel (tokens frescos) → master_url del source (para el mismo episodio)
        //    Nota: el source.url ES el master de este episodio; usarlo.
        // 2. si las URIs de segmentos locales coinciden con las primeras del nivel fresco → build híbrida
        //    (coincidencia = mismo uri absoluto para los primeros n segmentos)
        // 3. escribe {episode_dir}/playback.m3u8 y devuelve la ruta
        // 4. cualquier error → source.url
    }
}
```

`commands::play_episode` pasa a ser:
```rust
let source = state.anime.resolve_video(&slug, number).await?;
let url = state.buffer.resolve_playback_url(&source, &slug, number).await;
let player = state.player.lock().unwrap();
player.play(&slug, number, source_para_titulo, &title, start, url); // se añade parámetro url_final
```
El `PlayerService::play` recibe la URL final (local o remota); el resto del flujo no cambia.

## 2.3 Flujo de datos completo

```
[Reproduciendo ep.10 al 80%]
   buffer_loop (1s):
     PlayerState { slug=X, number=10, position/duration >= 80%, playing=true }
        → cola = [{X,11},{X,12},{X,13}]  (según count=3, acotado por total_episodes)
   download_job(X,11):
     resolve_video → master.m3u8 → nivel → [seg0..segN] (20% ≈ ceil(0.2*N))
     con throttle 5 Mbps y checks de pausa/cancelación
        → cache/{sha256(X)}/11/seg_0.ts ... + manifest.json
   [job 11 termina] → job 12 → job 13 (secuencial)
[Usuario reproduce ep.11]
   play_episode:
     source = resolve_video(11)          (nuevo token)
     url = resolve_playback_url(source,11):
        nivel fresco + manifest válido → escribe playback.m3u8 (seg_0..seg_k locales + tail remoto)
     player.play(url=playback.m3u8)  → mpv lee disco para el 20% inicial, red para el resto
```

## 2.4 Eventos Tauri

| Evento | Payload | Cadencia |
|---|---|---|
| `buffer://status` | `BufferStatus` | on change, máx 1/s |
| `player://state` (modificado) | `PlayerState` + campo `buffering: bool` | igual que hoy |

## 2.5 APIs IPC (nuevos comandos)

```rust
#[tauri::command] fn buffer_get_config(state) -> Result<BufferConfig, AppError>
#[tauri::command] fn buffer_set_config(state, config: BufferConfig) -> Result<BufferConfig, AppError>
#[tauri::command] fn buffer_get_status(state) -> Result<BufferStatus, AppError>
#[tauri::command] fn buffer_clear_cache(state) -> Result<u64, AppError>   // bytes liberados
#[tauri::command] fn buffer_pause(state, paused: bool) -> Result<(), AppError> // pausa manual
```

Registro en `invoke_handler` de `lib.rs`. Frontend: `api.bufferGetConfig()` etc.

## 2.6 Modelo de datos (migración 0002)

```sql
CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,               -- JSON (BufferConfig serializado)
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
```

`SettingsRepo` (nuevo, en `infra/repos.rs`): `get(key)`, `set(key, value)`, `all()`.
Carga en `setup()`: si `smart_buffer` no existe → se inserta con defaults. Clave única: `"smart_buffer"`.

## 2.7 Ciclo de vida y shutdown

- `BufferService::spawn`: crea `JoinSet` interno, hilo `kakasuma-buffer` con `rt.block_on`.
- `BufferCommand::Shutdown` en `RunEvent::Exit/ExitRequested` (lib.rs): aborta tasks del JoinSet (drop) y el bucle sale. Timeout de espera 2 s.
- `Drop` de `BufferService` aborta todo (seguridad ante cierre abrupto).
- Al **desactivar** (config): se abortan jobs activos y se vacía la cola (no la caché en disco, que se reutiliza). Al **limpiar caché**: borrado del directorio.

## 2.8 Restricciones de recursos (enforcement)

| Recurso | Límite | Mecanismo |
|---|---|---|
| Ancho de banda | `buffer_bandwidth_limit_mbps` | Token bucket (chunks 64 KiB) |
| Disco | `buffer_cache_limit_mb` | Contador atómico + purge LRU a 80 % |
| CPU | `buffer_cpu_threshold_percent` | `sysinfo` + auto-pausa |
| Concurrencia | 1 job activo | Cola secuencial |
| Progreso | ≥ `buffer_trigger_percent` | Trigger del bucle |
| Episodios | ≤ `buffer_episode_count` y ≤ `total_episodes` | Acotado por BD |
| % episodio | ≤ `buffer_percentage` (nunca 100) | `segment_range` |

## 2.9 Dependencias nuevas

- `reqwest` → activar feature `stream` (para throttle/pausa por chunk).
- `sysinfo = "0.33"` (o la versión estable compatible con rust-version 1.77; elegir la menor que compile).
- Ningún cambio en `mpv`, `sqlx`, `scraper`, etc.
