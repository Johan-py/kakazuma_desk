# Fase 1 — Análisis técnico: Background Smart Buffer

> Sistema opcional de precarga en segundo plano para Kakasuma.
> Objetivo: reducir microcortes, fluctuaciones de red y tiempos de espera al cambiar de episodio.
> NO es descarga completa de episodios ni sistema P2P.

---

## 1.1 Arquitectura actual relevante

### Flujo de reproducción (estado actual)

```
UI (play_episode) ─► IPC ─► commands::play_episode
                              │  state.anime.resolve_video(slug, number)
                              ▼
                       provider.resolve_video  (scraping JKAnime)
                              │  1. GET https://jkanime.net/{slug}/{number}/
                              │  2. extrae iframe jkplayer/umv?e=...
                              │  3. GET reproductor → <source src="...master.m3u8?token">
                              ▼
                       VideoSource { url: HLS } ─► PlayerService.play(url)
                              ▼
                       hilo "kakasuma-mpv" → libmpv loadfile(url)   [MPV baja HLS por su cuenta]
```

### Componentes clave (con referencias)

| Componente | Archivo | Rol | Observaciones para el buffer |
|---|---|---|---|
| `PlayerService` | `src-tauri/src/services/player.rs` | Hilo dedicado `kakasuma-mpv` ejecutando libmpv. Comandos por canal `std::sync::mpsc`, estado en `Arc<Mutex<PlayerState>>`, poll cada 1 s | No se debe tocar su bucle crítico. Expone `state` compartido que el buffer puede leer. No configura caché de mpv |
| `AnimeService::resolve_video` | `services/anime.rs:178` | Resuelve URL HLS por episodio (sin caché, expira) | Reutilizable por el buffer para episodios futuros |
| `HttpClient` | `infra/http.rs` | reqwest 0.12: keep-alive, HTTP/2, cookies, retry con backoff | Reutilizar; añadir método de streaming para segmentos |
| Cachés | `infra/cache.rs` | `TtlCache` (mem LRU) y `DiskCache` (JSON + SHA256 + TTL mtime) | No aptos para segmentos binarios; crear caché específica de segmentos |
| `Db` / repos | `infra/db.rs`, `infra/repos.rs` | SQLite WAL (sqlx), migraciones | `episode.video_url` nunca se rellena (el provider lo deja `None`) → hay que re-resolver cada episodio |
| `AppState` | `state.rs` | `{ anime, history, favorites, player: Mutex<PlayerService> }` | Integrar `BufferService` y `SettingsService` |
| Setup | `lib.rs` | Construye servicios, `player: Mutex<PlayerService>` | Punto único de wiring |
| `commands.rs` | — | Comandos Tauri (invoke_handler) | Añadir comandos de buffer/config |
| **Configuración** | **—** | **NO EXISTE ningún sistema de settings** (ni tabla, ni comando, ni UI) | **Hay que crearlo desde cero** |
| Frontend | `stores/useAppStore.ts`, `lib/api.ts`, `lib/types.ts` | Zustand + wrappers `invoke` + listeners `player://*` | Añadir estado de buffer y vista de ajustes |

### Modelo de datos actual

Tablas: `anime`, `episode`, `tag`, `anime_tag`, `watch_history`, `favorite_anime`.
Relevante: `episode(id_episode, id_anime, number, title, video_url, duration)` con `UNIQUE(id_anime, number)`.
`AnimeRepo::get_by_slug` + `episodes` dan el listado de episodios y `total_episodes` para acotar el buffer.

---

## 1.2 ¿Cuánto aporta MPV por sí solo?

`libmpv` ya usa `--cache=yes` para flujos de red de forma implícita, pero con topes por defecto:

| Opción | Default aprox. | Efecto |
|---|---|---|
| `cache=yes` | activo en red | Cache de demuxer en memoria |
| `cache-secs` | alto (según heurística) | Segundos "grabados" hacia delante |
| `demuxer-max-bytes` | ~128 MiB | Tope físico del cache adelante |
| `demuxer-max-back-bytes` | ~64 MiB (mitad) | Tope del cache hacia atrás |

**Beneficio real si se tunen** (siempre recomendado, costo cero):
- Absorbe fluctuaciones de red del **episodio en curso** (microcortes).
- Permite rebobinar dentro del tramo cacheado.

**Problemas que MPV por sí solo NO resuelve:**
1. **Inicio de episodios futuros**: al cambiar de episodio todo se baja de cero (resolución scraping + primera respuesta CDN + primeros segmentos). Es el mayor tiempo de espera percibido.
2. El cache de mpv es volátil (memoria) y se descarta al cerrar la app o cambiar de archivo.
3. El cache por defecto es acotado (~128 MiB) y no configurable por el usuario desde la app.
4. No precarga nada "mirando hacia delante" entre episodios.

**Conclusión**: el tuning de mpv (Fase 4.0) es **complementario**, no sustitutivo. Se implementan ambos.

---

## 1.3 Comparativa de estrategias de caché

### Opción A — Segmentos HLS individuales
Descargar los segmentos `.ts` del inicio del episodio futuro y, al reproducir, servir una **playlist híbrida** (locales primero + remotos después) a MPV.

| Ventajas | Desventajas |
|---|---|
| Control exacto del % (número de segmentos) | Requiere parser HLS (maestro, niveles, claves, rangos) |
| MPV lee del disco local: inmune a jitter en el tramo cacheado | Validación de frescura de tokens al reproducir |
| Sin puertos, sin servidor, sin superficie de seguridad | Variantes de calidad / cifrado AES-128 requieren manejo |
| Limpieza LRU por segmento (granular) | Degradar a reproducción remota si el HLS es exótico |

### Opción B — Archivo temporal por episodio
Descargar un `.ts` concatenado (parcial o completo) por episodio y reproducir el archivo.

| Ventajas | Desventajas |
|---|---|
| MPV reproduce un archivo plano (máxima robustez) | **Sin transición local→remoto**: un `.ts` parcial se corta al final y no continúa por red |
| | Disco = copia completa aunque el usuario no vea el episodio |
| | Si el usuario reproduce el episodio desde el minuto 30, el buffer del inicio es inútil |
| | Cancelar a medias deja un archivo inservible |

### Opción C — Proxy local de streaming
Servidor HTTP local (`127.0.0.1`) que resuelve el HLS remoto y sirve segmentos desde caché (local) o red (remote).

| Ventajas | Desventajas |
|---|---|
| Híbrido transparente, robusto ante HLS exóticos | Proceso servidor: puerto, binding, seguridad, ciclo de vida |
| URL estable para MPV | Más código (rutas, range requests, headers, caché) |
| | Sobrecarga por cada segmento (loopback + deserialización) |
| | Más difícil de cancelar/limpiar y de acotar recursos |

---

## 1.4 Recomendación final

**Opción A (segmentos HLS + playlist híbrida local-first).**

Justificación técnica:
1. **Es la única que cumple "prioridad inferior" y "nunca afectar a la reproducción"** de forma natural: MPV lee disco local cuando existe buffer; si algo falla, se reproduce la URL remota original sin cambiar el flujo actual.
2. **Menor superficie**: sin servidor, sin puertos; el único punto de contacto es `BufferService.resolve_playback_url()` antes de `player.play()`.
3. **Se integra con libmpv tal cual**: `loadfile(/abs/path/xxx.m3u8)` — mpv detecta HLS por extensión/contenido.
4. **Degradación elegante** (requisito "nunca afectar negativamente"): si el HLS es vivo (sin `#EXT-X-ENDLIST`), usa `#EXT-X-BYTERANGE`, o falla la validación de frescura → se reproduce remoto.
5. **Uso eficiente de disco**: solo se guarda el % configurado, por segmento, con evicción LRU por bytes.

**Limitaciones aceptadas (documentadas):**
- La variante de calidad cacheada es la primera del maestro m3u8 (en JKAnime suele haber una única). Al reproducir el episodio con buffer, la calidad es la de esa variante durante todo el episodio (playlist híbrida completa).
- Los tokens de los segmentos remotos caducan; por eso al reproducir se **re-resuelve y se revalida** la playlist. Si los segmentos locales no coinciden, se reproduce remoto.
- Los episodios futuros requieren scraping (1 request de página + 1 de reproductor cada uno), igual que haría el usuario al pulsar play.
- El buffer solo se activa durante reproducción activa de una serie (trigger por progreso).

---

## 1.5 Riesgos y mitigaciones

| # | Riesgo | Impacto | Mitigación |
|---|---|---|---|
| 1 | URLs HLS con token expirable | Reproducción del tail remoto falla | Re-resolver en `resolve_playback_url`; validar coincidencia de segmentos locales vs remotos; fallback a remoto |
| 2 | Master m3u8 con varias variantes | Calidad inesperada / salto de bitrate | Elegir la 1.ª variante estable; documentado |
| 3 | Cifrado `#EXT-X-KEY` (AES-128) | Segmentos locales ilegibles | Cachear clave, reescribir URI a local; si no se puede → abortar y remoto |
| 4 | `#EXT-X-BYTERANGE` o playlists vivas | Caché inviable | Detectar y abortar (estado `unsupported`) |
| 5 | Bloqueo del hilo mpv / IPC / UI | Microcortes, UI congelada | Buffer 100 % async (Tokio); nunca bloquear `Mutex` del player; solo lectura del estado |
| 6 | Saturación de red | Afecta a mpv | Token bucket por ancho de banda + 1 solo job activo + pausa si mpv hace buffering |
| 7 | CPU alta | Laptop lenta | `sysinfo`, auto-pausa por umbral configurable |
| 8 | Disco lleno | LLenado de caché | Límite por bytes + purge LRU + auto-pausa |
| 9 | Fugas de memoria / tareas huérfanas | App lenta | `JoinSet` + `AbortHandle`, shutdown limpio en `Exit` |
| 10 | No existe sistema de config | No se puede persistir | Nueva tabla `settings` + `SettingsService` + IPC |
| 11 | Concurrencia de escritura/purge | Corrupción de caché | Índice en memoria con `Mutex` corto; escrituras atómicas (`tmp`+`rename`) |
| 12 | Pre-cargar episodios que no existen | Resolve falla en bucle | Acotar por `total_episodes`/`episodes` de la BD |
| 13 | Cambio de configuración en caliente | Tareas a mitad | Re-aplicar config: abortar y re-encolar según nuevos parámetros |
