# Arquitectura de Kakazuma

Kakazuma es una aplicación de escritorio para ver anime en español construida
con **Tauri 2** (Rust en el backend) y **React 18 + TypeScript** en el
frontend. No aloja contenido: actúa como una interfaz nativa sobre sitios de
streaming (actualmente **jkanime.net**) a través de scraping.

Este documento explica, de forma descriptiva y de arriba hacia abajo, cómo está
organizada toda la lógica del proyecto.

---

## 1. Visión general

La aplicación tiene dos procesos distintos conviviendo en un mismo binario:

1. **Núcleo Rust (Tauri)** — todo el trabajo pesado:
   - Scraping del proveedor (búsqueda, catálogo, detalle, episodios, video).
   - Persistencia en SQLite (animes, episodios, tags, historial, favoritos, settings).
   - Cachés en memoria y en disco.
   - Reproductor **libmpv** en su propio hilo.
   - **Smart Buffer**: precarga en segundo plano de segmentos HLS de episodios
     futuros, servidos luego a mpv desde un servidor HTTP local.
2. **Frontend React (WebView)** — la interfaz:
   - Se comunica con el backend mediante `invoke()` (llamadas request/response).
   - Recibe eventos en tiempo real (estado del reproductor, progreso, estado del
     buffer) vía `listen()` sobre canales tipo `player://*` y `buffer://*`.

```
┌────────────────────────────────────────────────────────────────┐
│                        FRONTEND (React)                        │
│  Zustand store ── components ── lib/api.ts (invoke/listen)     │
└───────────────────────────┬────────────────────────────────────┘
                            │  invoke()                ▲
                            │  eventos                 │ player://* , buffer://*
                            ▼                          │
┌────────────────────────────────────────────────────────────────┐
│                     NÚCLEO (Tauri / Rust)                       │
│                                                                │
│  commands.rs  (puente IPC)                                     │
│       │                                                        │
│       ▼                                                        │
│  services/                                                      │
│   ├ anime.rs   ├ player.rs  ├ buffer.rs  ├ history.rs          │
│   └ favorite.rs                                               │
│       │                                                        │
│       ├── provider/  (trait Provider + jkanime scraping)      │
│       └── infra/     (db, http, cache, hls, segcache,         │
│                        segserver)                              │
└────────────────────────────────────────────────────────────────┘
```

---

## 2. Stack tecnológico

| Capa | Tecnología | Uso |
|---|---|---|
| Frontend | React 18, TypeScript, Vite | Interfaz de usuario |
| Estilos | TailwindCSS | UI |
| Estado frontend | Zustand 5 | Store global reactiva |
| IPC | `@tauri-apps/api` | `invoke()` y `listen()` |
| Backend | Rust + Tauri 2 | Proceso nativo |
| Runtime async | tokio | Tareas concurrentes (red, buffer) |
| HTTP | reqwest (HTTP/2, cookies, retry) | Scraping y descargas |
| BD | SQLite + sqlx (WAL, migraciones) | Persistencia local |
| Reproducción | libmpv (`mpv-rs`) | Reproductor en hilo propio |
| Scraping | scraper + regex | Parseo de HTML de jkanime.net |
| HLS | parsing y descarga propios | Smart Buffer |

---

## 3. Estructura del proyecto

```
kakazuma_desk/
├── index.html                  Entrada HTML del frontend (Vite)
├── src/                        FRONTEND (React)
│   ├── main.tsx                Bootstrap de React
│   ├── App.tsx                 Rutas de vistas + init global
│   ├── index.css               Tailwind
│   ├── lib/
│   │   ├── types.ts            Tipos compartidos (espejo del domain.rs)
│   │   ├── api.ts              Envoltorio de invoke() para cada comando
│   │   └── global.d.ts         Tipos globales (unlisteners)
│   ├── stores/
│   │   └── useAppStore.ts      Store Zustand: estado + acciones + eventos
│   └── components/             Vistas (Home, Catalog, Search, Detail,
│                               Favorites, History, PlayerOverlay, Navbar…)
│
├── src-tauri/                  BACKEND (Rust)
│   ├── Cargo.toml              Dependencias
│   ├── tauri.conf.json         Config de la ventana y del build
│   ├── capabilities/default.json  Permisos Tauri del frontend
│   ├── migrations/             Migraciones SQL (0001_init, 0002_settings)
│   ├── icons/                  Iconos
│   └── src/
│       ├── main.rs             Punto de entrada (llama a lib::run)
│       ├── lib.rs              Arranque: setup, wiring de servicios, handlers
│       ├── state.rs            AppState: estado global compartido
│       ├── commands.rs         Comandos IPC (tauri::command)
│       ├── domain.rs           Modelos de datos (serde)
│       ├── error.rs            AppError + AppResult
│       ├── settings.rs         BufferConfig + SettingsService
│       ├── services/           Lógica de negocio
│       │   ├── anime.rs        Búsqueda, catálogo, detalle, video
│       │   ├── player.rs       Hilo libmpv + guardado de progreso
│       │   ├── buffer.rs       Smart Buffer (bucle + jobs de descarga)
│       │   ├── history.rs      Historial / continuar viendo
│       │   └── favorite.rs     Favoritos
│       ├── provider/           Capa de scraping
│       │   ├── mod.rs          Trait Provider (contrato extensible)
│       │   ├── jkanime.rs      Implementación JKAnime
│       │   └── jkanime_parse.rs  Parsing HTML (scraper + regex)
│       └── infra/              Infraestructura
│           ├── db.rs           Pool SQLite + migraciones
│           ├── repos.rs        Accesos a BD (Anime/Episode/Tag/History/…)
│           ├── http.rs         HttpClient (retry, backoff, cookies)
│           ├── cache.rs        TtlCache (mem) + DiskCache + CacheRegistry
│           ├── hls.rs          Parseo HLS, descarga, playlist híbrida
│           ├── segcache.rs     Caché de segmentos en disco (LRU)
│           └── segserver.rs    Servidor HTTP local para servir segmentos
│
├── docs/buffer/                Documentación de diseño del Smart Buffer
├── legacy/                     CLI original (referencia histórica)
├── package.json                Frontend + scripts (vite, tauri)
└── README.md
```

---

## 4. Backend: arranque y wiring (`lib.rs`)

El flujo de inicialización de la aplicación está en `src-tauri/src/lib.rs::run()`:

1. Se configura el logging (`tracing`).
2. En `setup` se obtiene el directorio de datos (`app_data_dir`).
3. Se conecta a SQLite (`Db::connect`) y se ejecutan las migraciones.
4. Se construye el **HttpClient** compartido.
5. Se crea el **proveedor** (`JKAnimeProvider`) como `Arc<dyn Provider>`.
6. Se inicializan las cachés (disco + TTL).
7. Se crean los servicios:
   - `AnimeService` (provider + caché + BD)
   - `HistoryService` y `FavoriteService` (BD)
   - `SettingsService` (config persistente)
   - `PlayerService::spawn` — arranca el hilo de libmpv
   - `BufferService::spawn` — arranca el hilo del Smart Buffer y el servidor
     local de segmentos.
8. Todo queda registrado en `AppState` (`app_handle.manage`).
9. Se registran todos los comandos IPC.
10. En la salida (`Exit`/`ExitRequested`) se detiene el reproductor y se apaga
    el buffer limpiamente.

> Nota: los servicios `Player` y `Buffer` **no bloquean el main thread**: cada
> uno vive en su propio hilo y se comunican por canales mpsc y estado
> compartido.

---

## 5. Estado global (`state.rs`)

`AppState` es el "contenedor" que Tauri inyecta en cada comando:

```rust
pub struct AppState {
    pub anime: Arc<AnimeService>,
    pub history: HistoryService,
    pub favorites: FavoriteService,
    pub player: Mutex<PlayerService>,
    pub settings: Arc<SettingsService>,
    pub buffer: BufferService,
}
```

- Los servicios son `Send + Sync`, así que pueden compartirse entre hilos.
- `player` se envuelve en `Mutex` porque los comandos se ejecutan en hilos del
  runtime de Tauri y acceden a él desde el hilo del reproductor.

---

## 6. Capa de comandos (IPC)

`commands.rs` expone ~30 comandos anotados con `#[tauri::command]`. Cada uno
toma `State<'_, AppState>` y delega en el servicio correspondiente. Se agrupan
en dominios:

| Grupo | Comandos | Servicio |
|---|---|---|
| Catálogo | `search_anime`, `get_catalog`, `get_tags`, `get_recent`, `get_recommended`, `get_anime_detail` | `anime` |
| Video | `resolve_video` | `anime` |
| Reproducción | `play_episode`, `player_*` (pause, seek, speed, volume, mute, fullscreen, stop, get_state) | `player` |
| Historial | `continue_watching`, `save_progress`, `clear_history` | `history` |
| Favoritos | `list_favorites`, `add_favorite`, `remove_favorite`, `is_favorite` | `favorites` |
| Buffer | `buffer_get_config`, `buffer_set_config`, `buffer_get_status`, `buffer_clear_cache`, `buffer_pause` | `buffer` + `settings` |

Los comandos de control del reproductor no esperan resultado: envían un
`PlayerCommand` por canal y vuelven inmediatamente.

---

## 7. Modelo de datos (`domain.rs`)

Son las estructuras serde que viajan entre Rust y TypeScript:

- **Anime**: slug, nombre, sinopsis, temporada, estado, carátula, nº episodios,
  tipo, URL y géneros.
- **Episode**: número, título, `video_url`, duración.
- **Tag**: género/categoría con id y descripción.
- **VideoSource**: URL del stream (m3u8) + calidad + subtítulos.
- **AnimeDetail**: anime + episodios + tags (lo que ve la vista de detalle).
- **WatchHistoryEntry**: anime + episodio + progreso + fechas (continuar viendo).
- **FavoriteEntry**: anime + fecha de añadido.
- **CatalogFilter / CatalogPage**: filtros y paginación del directorio.

> Los tipos del frontend (`src/lib/types.ts`) son el espejo exacto de estos
> structs, de modo que lo que serializa Rust es lo que tipa TypeScript.

---

## 8. Capa de servicios

### 8.1 `AnimeService` — catálogo y contenido

Conecta proveedor + caché + BD. Estrategia de lectura en **tres niveles**:

1. Caché en memoria (`TtlCache`, LRU, TTL 15 min).
2. Caché en disco (`DiskCache`, JSON, TTL por dominio).
3. Proveedor (red).

En `get_anime_detail`, además de devolver el detalle, **persiste** el anime y sus
episodios/tags en SQLite (upsert por slug), lo que alimenta el historial y los
favoritos. `resolve_video` **no se cachea** porque las URLs de stream expiran.

### 8.2 `PlayerService` — reproductor libmpv

Es un **hilo dedicado** (`kakazuma-mpv`). Estructura:

- **Entrada**: canal mpsc de `PlayerCommand` (Play, Pause, Seek, SetSpeed, ...).
- **Salida**: `PlayerState` compartida en `Arc<Mutex>` + eventos Tauri
  (`player://state`, `player://progress`, `player://end`, `player://error`).

El bucle del hilo:

1. Drena los comandos pendientes (`try_recv`).
2. Hace *polling* cada 1 s de `time-pos`, `duration`, `pause`,
   `paused-for-cache` y actualiza el estado compartido.
3. Cada **10 s** guarda el progreso en BD y emite `player://progress`
   (siempre que la duración > 0).
4. Procesa eventos de mpv (`StartFile`, `FileLoaded`, `EndFile`, `Idle`) para
   reanudar posición, detectar fin y limpiar estado.

También configura libmpv con **cache de demuxer propio** (300 s, 512 MiB de
máximo hacia adelante) que complementa al Smart Buffer suavizando las
fluctuaciones de red del episodio en curso.

### 8.3 `BufferService` — Smart Buffer

Vive en su propio hilo (`kakazuma-buffer`) corriendo un bucle asíncrono.
Se detalla en la sección 11.

### 8.4 `HistoryService` / `FavoriteService` — persistencia

Delgados: delegan en `HistoryRepo` / `FavoriteRepo`. `continue_watching`
devuelve el **último episodio visto por cada anime**, ordenado por fecha.

### 8.5 `SettingsService` — configuración

Guarda `BufferConfig` como JSON en la tabla `settings` (clave `smart_buffer`).
Mantiene una copia en memoria (`RwLock`) para lecturas rápidas sin IO. Todos los
valores se sancionan con `BufferConfig::sanitized()` antes de persistir.

---

## 9. Capa de proveedores (`provider/`)

El contrato es el trait `Provider` (async_trait):

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(...) -> AppResult<Vec<Anime>>;
    async fn get_anime(...) -> AppResult<Anime>;
    async fn get_episodes(...) -> AppResult<Vec<Episode>>;
    async fn resolve_video(...) -> AppResult<VideoSource>;
    async fn catalog(...) -> AppResult<CatalogPage>;
    async fn recent(...) -> AppResult<Vec<Anime>>;
    async fn recommended(...) -> AppResult<Vec<Anime>>;
    async fn genres(...) -> AppResult<Vec<Tag>>;
}
```

`JKAnimeProvider` implementa el scraping:

- **search**: `/buscar/{query}/` → parsea `.anime__item`.
- **get_anime**: página del slug → parsea `.anime_info`, `.anime_data` y
  `og:image`.
- **get_episodes**: usa el endpoint AJAX `/ajax/episodes/{id}/` con **token
  CSRF** (extraído con regex del HTML) y `POST` de formulario; paga la
  paginación en paralelo (JoinSet, máx. 6 tareas).
- **resolve_video**: página del episodio → encuentra el iframe `jkplayer/umv` →
  carga esa página → extrae la URL del `<source>` (regex). El resultado es la
  URL **maestra m3u8**.
- **catalog**: `/directorio` con filtros → parsea el JSON embebido en
  `var animes = {...}`.
- **recent/recommended**: secciones de la home ("Animes recientes", "Top animes").
- **genres**: options del `<select name="genero">` en `/directorio`.

Un detalle de robustez: `page_info` cachea el id numérico y el token CSRF por
slug durante 10 minutos, evitando re-fetchear la página en cada llamada.

> La idea es que **cualquier otro proveedor** (AnimeFLV, TioAnime, Crunchyroll,
> etc.) implemente el mismo trait sin tocar servicios, comandos ni UI.

---

## 10. Capa de infraestructura (`infra/`)

### 10.1 `db.rs` — SQLite

Pool de conexiones (1–5) con:
- `journal_mode = WAL`
- `foreign_keys = ON`
- `busy_timeout = 5 s`
- `auto_vacuum = incremental`

Las migraciones viven en `migrations/` y se aplican al arranque con
`sqlx::migrate!`. Esquema:

| Tabla | Propósito |
|---|---|
| `anime` | Datos del anime (slug único) |
| `episode` | Episodios (UNIQUE(anime, number)) |
| `tag` | Géneros |
| `anime_tag` | Relación N:M anime ↔ tag |
| `watch_history` | Progreso de reproducción por episodio |
| `favorite_anime` | Favoritos |
| `settings` | Config clave-valor (smart_buffer JSON) |

### 10.2 `repos.rs` — repositorios

Accesos SQL explícitos y tipados:
- `AnimeRepo`: upsert por slug (RETURNING id), consultas por slug/id, episodios.
- `EpisodeRepo`: `upsert_many` en transacción, `set_video_url`.
- `TagRepo`: `ensure`, `set_for_anime`, `all`.
- `HistoryRepo`: `upsert` (UPDATE else INSERT), `continue_watching` (subconsulta
  para el último episodio por anime), `clear`.
- `FavoriteRepo`: `add/remove/is_favorite/list`.
- `SettingsRepo`: get/set clave-valor.

### 10.3 `http.rs` — cliente HTTP

`HttpClient` envuelve reqwest con:
- User-Agent de navegador, `Accept-Language: es-ES`.
- **Cookie store** habilitado (clave para el scraping autenticado por cookies).
- Timeouts de conexión (10 s) y petición (45 s).
- **Retry con backoff exponencial + jitter** en GET (método `get_text`,
  `get_bytes`, `get_json`) y en `post_form`. No reintenta en `get_response`
  (streaming), donde los reintentos los gestiona el llamante.
- HTTP/2 adaptativo y keep-alive.
- Helper `resolve_url` y `CsrfSession`/`csrf_headers` para peticiones AJAX.

### 10.4 `cache.rs` — cachés

- `TtlCache<V>`: LRU en memoria con TTL por entrada.
- `DiskCache`: archivos JSON con clave = SHA-256 del nombre; TTL por mtime;
  escritura atómica (tmp + rename).
- `CacheRegistry`: TTLs por dominio de caché (p. ej. géneros 24 h).

### 10.5 `hls.rs` — HLS

- **parse_master**: detecta variantes (`#EXT-X-STREAM-INF`) y elige la de mayor
  resolución (desempate por bandwidth).
- **parse_level**: extrae segmentos, duraciones, target duration, claves
  AES-128 (`#EXT-X-KEY`) y detecta playlists **no soportadas** para el buffer:
  vivas (sin `#EXT-X-ENDLIST`), `#EXT-X-BYTERANGE` y fMP4 (`#EXT-X-MAP`).
- **`segment_range`**: nº de segmentos a precargar según porcentaje
  (nunca 100 %, al menos 1, nunca todos).
- **`build_hybrid_playlist`**: playlist m3u8 que combina **segmentos locales**
  (rutas absolutas en disco) con el **tail remoto** y la clave de cifrado del
  tail. Es lo que se sirve a mpv.
- **`download_segment`**: descarga un segmento con throttling por chunks de
  64 KiB, reintentos y chequeos de pausa/cancelación.
- **`TokenBucket`**: limitador de ancho de banda (Mbps → bytes/s).
- `key_uris_match` compara claves ignorando la query (tokens que expiran).

### 10.6 `segcache.rs` — caché de segmentos

Estructura de directorios:
```
{dir}/{sha256(slug)}/{number}/seg_{i:04}.ts
{dir}/{sha256(slug)}/{number}/manifest.json
```
- Contador atómico de bytes totales.
- **Evicción LRU por mtime** cuando se supera el límite (`enforce_limit`),
  purgando hasta el 80 % del límite.
- `remove_episode` (cuando el episodio ya se consumió al 90 %), `clear`.

### 10.7 `segserver.rs` — servidor local de segmentos

Servidor HTTP mínimo en `127.0.0.1` con puerto efímero:
- Sirve rutas `/buffer/<hash>/<number>/<archivo>`.
- **Protección contra path traversal** (`..` y canonical check dentro de la raíz).
- Soporta `Range` (mpv usa peticiones parciales) → responde `206 Partial Content`.
- MIME correcto para `.m3u8`, `.ts`, `.json`.
- Se apaga junto con el buffer.

Con esto, cuando un episodio ya está precargado, la playlist híbrida apunta a
segmentos locales que mpv descarga a velocidad de disco en lugar de la red.

---

## 11. Smart Buffer a fondo

El Smart Buffer reduce microcortes y esperas al cambiar de episodio.

### 11.1 Bucle principal (`buffer_loop`)

Cada ~1 s:

1. Atiende comandos de control (cambio de config → aborta jobs y reaplaza;
   clear cache; shutdown).
2. Lee el estado del reproductor (**solo lectura**, no bloquea mpv).
3. Calcula condiciones de pausa en un **bitmask** `AtomicU64`:
   - `PAUSE_BUFFERING` (mpv en buffering)
   - `PAUSE_NOT_PLAYING` (cargado pero pausado)
   - `PAUSE_CPU` (uso de CPU global > umbral)
   - `PAUSE_DISK` (caché por encima del límite)
   - `PAUSE_NETWORK` (backoff tras errores consecutivos)
   - `PAUSE_MANUAL` (pausa desde la UI)
4. Gestiona el **ancla**: el episodio en reproducción. Al cambiar de ancla,
   aborta todos los jobs y regenera la cola.
5. Cuando el progreso del ancla supera `buffer_trigger_percent` (por defecto
   80 %), encola los `buffer_episode_count` episodios siguientes (una sola vez
   por ancla).
6. Si hay cola, no hay job activo y no hay pausa → arranca un **job de
   descarga** en un `JoinSet` de tokio.
7. Recolecta resultados; 3 errores consecutivos → pausa de red con backoff de
   10 s.
8. Emite `buffer://status` solo cuando el estado cambió.

### 11.2 Job de descarga (`download_job`)

1. `anime.resolve_video(slug, number)` → URL del stream.
2. `HlsClient.resolve_level` → playlist de nivel (rechaza live/byterange/fMP4).
3. Calcula los segmentos a precargar con `segment_range(percent)`.
4. Descarga secuencialmente cada segmento con throttling, pausas y
   cancelación por generación.
5. Escribe el `manifest.json` (URI de cada segmento, duración, clave) de forma
   atómica (tmp + rename).
6. Reporta progreso en `BufferStatus`.

### 11.3 Reproducción local-first (`resolve_playback_url`)

Al pedir `play_episode`, el buffer decide la URL final:

1. Si el buffer está desactivado o no hay manifest → URL remota original.
2. **Revalida el HLS** en vivo y compara:
   - que el número de segmentos frescos sea ≥ los cacheados,
   - que cada URI coincida (los tokens/URLs expiran),
   - coherencia de la clave AES-128.
3. Si todo coincide, construye la playlist híbrida (local + tail remoto) y la
   escribe como `playback.m3u8`.
4. Devuelve `http://127.0.0.1:<puerto>/buffer/<hash>/<num>/playback.m3u8`.

Si cualquier chequeo falla, **fallback silencioso** a la URL remota: la
reproducción nunca se rompe por el buffer.

### 11.4 Configuración (`BufferConfig`)

| Campo | Default | Rango sanitizado |
|---|---|---|
| `smart_buffer_enabled` | true | — |
| `buffer_episode_count` | 1 | 0–5 |
| `buffer_percentage` | 20 | 5–90 |
| `buffer_cache_limit_mb` | 1000 | 100–10000 |
| `buffer_bandwidth_limit_mbps` | 5 | 1–100 |
| `buffer_trigger_percent` | 80 | 50–95 |
| `buffer_cpu_threshold_percent` | 70 | 20–100 |

---

## 12. Frontend (React + Zustand)

### 12.1 Flujo de datos

- `src/lib/api.ts` envuelve `invoke()` de Tauri en una función `call<T>` que
  lanza `Error` con el mensaje del backend. Expone un objeto `api` con un método
  por comando.
- `src/stores/useAppStore.ts` es la única fuente de verdad del frontend.
  - Acciones que llaman a `api.*` y rellenan el store.
  - `initPlayer` y `initBuffer` registran los **listeners** de eventos Tauri
    (`player://state`, `player://progress`, `player://end`, `player://error`,
    `buffer://status`) y los guardan en `window.__unlisteners`.
- Los componentes (`App.tsx` + vistas) solo leen del store y disparan acciones.

### 12.2 Vistas

| Vista | Componente | Contenido |
|---|---|---|
| Inicio | `HomeView` | Carrousels de recientes y recomendados |
| Catálogo | `CatalogView` | Filtros (género, demografía, temporada, tipo, estado, año) + paginación |
| Búsqueda | `SearchView` | Resultados por consulta |
| Detalle | `DetailView` | Info + sinopsis + géneros + lista de episodios |
| Favoritos | `FavoritesView` | Grid de animes marcados |
| Historial | `HistoryView` | "Continuar viendo" con progreso |
| Overlay | `PlayerOverlay` | Controles del reproductor en pantalla (play/pause, seek, velocidad, volumen, fullscreen) |

### 12.3 Ciclo de vida

En `App.tsx`, al montar: `initPlayer()`, `initBuffer()`, `loadTags()`,
`loadHome()`, `loadHistory()`, `loadFavorites()`. La vista activa se decide por
`view` del store (sin router).

---

## 13. Flujo de reproducción completo (ejemplo)

1. El usuario abre un anime → `openDetail(slug)` → `get_anime_detail`.
   El backend scrapea, **persiste** anime+episodios+tags en SQLite y cachea.
2. El usuario pulsa un episodio → `playEpisode(slug, number, title, start)`:
   - `resolve_video` → URL maestra m3u8.
   - `buffer.resolve_playback_url` → URL local-first (o remota si el buffer
     no aplica).
   - `player.play(...)` → `PlayerCommand::Play` por canal.
3. El hilo mpv carga el stream, reanuda desde `start` si procede, y empieza a
   emitir `player://state` cada segundo.
4. Cada 10 s se guarda el progreso (`HistoryRepo::upsert`) y se emite
   `player://progress`.
5. Cuando el progreso llega al 80 %, el buffer encola y descarga los siguientes
   episodios.
6. Al terminar el episodio (`player://end`), se limpia el estado, se guarda el
   progreso final y el frontend refresca el historial.
7. Al salir de la app, se manda `Stop` al reproductor y `shutdown` al buffer.

---

## 14. Manejo de errores

- `AppError` (thiserror) tipa cada dominio: red, proveedor, BD, caché,
  reproductor, no encontrado, config, no soportado.
- Se **serializa como string** con prefijo de código (`http_error: ...`,
  `provider_error: ...`) para que Tauri lo entregue al frontend como texto.
- El frontend convierte el string en `Error` y la UI muestra mensajes.

Estrategias defensivas clave:
- Cachés con TTL y escritura atómica (tmp + rename).
- Fallback del buffer a la URL remota si la validación falla.
- Retry con backoff en red; pausa por CPU/disco/red del buffer.
- `AppState` con servicios `Send + Sync`; mutexes cortos.

---

## 15. Diagrama de módulos

```
                           ┌──────────────┐
                           │   lib.rs     │  wiring + setup
                           └──────┬───────┘
                                  │
              ┌───────────────────┼────────────────────┐
              ▼                   ▼                    ▼
      ┌─────────────┐    ┌─────────────┐      ┌─────────────┐
      │ commands.rs │    │  AppState   │      │  shutdown   │
      │  (IPC)      │    │ (servicios) │      └─────────────┘
      └──────┬──────┘    └──────┬──────┘
             │                  │
             │   ┌──────────────┼──────────────────────────────┐
             │   ▼              ▼              ▼               ▼
             │   AnimeService   PlayerService   BufferService  History/Favorite
             │   │                │  (hilo mpv)   │ (hilo + segserver)
             │   │                │               │
             │   ▼                ▼               ▼
             │   Provider ──► JKAnimeProvider     │
             │   │               │ (scraping)     │
             │   │               ▼                │
             │   │         jkanime_parse.rs       │
             │   ▼                               ▼
             │   Infra: http.rs  hls.rs  segcache.rs  segserver.rs
             │   cache.rs        db.rs   repos.rs       settings.rs
             └─────────────────────────────────────────────
```

---

## 16. Documentación complementaria

- `docs/buffer/fase1-analisis.md` — análisis del problema del buffering.
- `docs/buffer/fase2-diseno.md` — diseño del Smart Buffer.
- `docs/buffer/fase3-plan.md` — plan de implementación.
- `legacy/` — CLI original que dio origen al proyecto.
