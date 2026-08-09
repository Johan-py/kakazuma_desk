# Fase 3 — Plan de implementación paso a paso

Orden de trabajo con verificación incremental. Cada paso compila.

## Paso 0 — Tuning de MPV (independiente, bajo riesgo)
- `services/player.rs`: setear en la creación de mpv:
  - `cache=yes`
  - `cache-secs=300`
  - `demuxer-max-bytes=536870912` (512 MiB)
  - `demuxer-max-back-bytes=134217728` (128 MiB)
  - `demuxer-readahead-secs=120`
- Añadir `buffering: bool` a `PlayerState` y al poll (propiedad `paused-for-cache`).
- **Verificar**: `cargo check`.

## Paso 1 — Settings (config persistente)
- Migración `0002_settings.sql` (tabla `settings`).
- `SettingsRepo` en `infra/repos.rs` (`get/set/all`).
- `settings.rs`: `BufferConfig` + `SettingsService` (carga en setup, validación de rangos, `set_config` persistente).
- Integrar en `setup()` de `lib.rs`.
- Comandos IPC `buffer_get_config` / `buffer_set_config`.
- **Verificar**: `cargo check` + `cargo test`.

## Paso 2 — infra/hls.rs (parser y cliente HLS)
- Parseo de playlists maestro/nivel (scraper no; parsing de texto manual, sin regex costosos):
  - `#EXT-X-STREAM-INF` → variantes; elegir 1.ª estable (con RESOLUTION o BANDWIDTH).
  - `#EXTINF`, URIs de segmento, `#EXT-X-KEY`, `#EXT-X-BYTERANGE`, `#EXT-X-ENDLIST`.
  - Resolver URIs relativas contra `base_url`.
- `resolve_level`, `segment_range`, `build_hybrid_playlist`.
- Tests unitarios con playlists de ejemplo.
- **Verificar**: `cargo test` (nuevos tests del parser).

## Paso 3 — infra/segcache.rs (caché de segmentos)
- Directorio `{app_data}/buffer`, índice por `sha256(slug)/number/seg_i.ts`.
- Contador de bytes (`AtomicU64`), `enforce_limit` (purge por mtime hasta 80 % del límite).
- Escritura atómica (`tmp` + `rename`), `clear()`.
- Tests unitarios con directorio temporal.
- **Verificar**: `cargo test`.

## Paso 4 — services/buffer.rs (servicio y worker)
- `BufferCommand` canal de control; hilo `kakazuma-buffer` con `rt.block_on(buffer_loop)`.
- Bucle 1 s: lectura de `PlayerState`, detección de trigger, generación de cola acotada, arranque de 1 `download_job`, condiciones de pausa (bitmask), emisión de `buffer://status`.
- `download_job`: resolve_video → nivel → rango → descarga secuencial con token bucket (reqwest `stream`) → manifest.json.
- `resolve_playback_url`: validación de frescura + playlist híbrida + fallback remoto.
- `BufferStatus` observable.
- **Verificar**: `cargo check`.

## Paso 5 — Wiring backend
- `state.rs`: añadir `settings` y `buffer`.
- `lib.rs`: construir servicios, registrar comandos, `Shutdown` en `Exit/ExitRequested`.
- `commands.rs`: `buffer_*` (config, status, clear, pause).
- `commands::play_episode` → usar `buffer.resolve_playback_url`.
- `PlayerService::play` → aceptar URL final.
- **Verificar**: `cargo check`.

## Paso 6 — Frontend
- `types.ts`: `BufferConfig`, `BufferStatus`, `BufferedEpisode`; `PlayerState.buffering`.
- `api.ts`: wrappers `bufferGetConfig`, `bufferSetConfig`, `bufferGetStatus`, `bufferClearCache`, `bufferPause`.
- `useAppStore.ts`: estado de buffer, listener `buffer://status`, acciones.
- `SettingsView.tsx` (nuevo tab "Ajustes"): toggle habilitar, select episodios (0-5), select % (10/20/30/50), select caché (500MB-5GB), select banda (1-10 Mbps), trigger y CPU; indicador de estado actual (activo/pausa/motivo, bytes en caché).
- `Navbar.tsx`: añadir tab "Ajustes".
- **Verificar**: `tsc` (npm run build).

## Paso 7 — Verificación final y pruebas manuales
- `cargo test` (parser HLS, segcache, settings).
- `cargo check` en release.
- `npm run build` (TypeScript).
- Pruebas manuales sugeridas:
  1. Reproducir ep.10 → llegar al 80 % → comprobar `buffer://status` con cola 11,12,13.
  2. Pausar reproducción → buffer en pausa (reason `not_playing`).
  3. Poner red limitada (simular con limit 1 Mbps) → el throttle respeta el límite.
  4. Reproducir ep.11 → arranque local (playlist híbrida).
  5. Cerrar app a mitad de descarga → sin tareas huérfanas (logs limpios).

## Criterios de aceptación
- [ ] Reproducción actual nunca se ve afectada (mpv conserva la red; buffer solo cuando hay margen).
- [ ] El buffer se activa solo tras el trigger configurable y se limita a `buffer_episode_count` episodios.
- [ ] El % precacheado nunca llega a 100 % (limitado por `buffer_percentage`).
- [ ] Shutdown limpio: al salir, todas las tareas abortadas.
- [ ] Configuración persistente y aplicable en caliente.
- [ ] Todos los límites (banda, disco, CPU, concurrencia) funcionan.
