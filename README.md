# Kakasuma

Aplicación de escritorio para ver anime en español. Evolución del CLI original
(`legacy/`) a una app nativa construida con **Tauri 2** (Rust) + **React** (TypeScript).

> Interfaz para sitios de streaming. No aloja contenido propio. Fuente: `jkanime.net`

## Características

- **Búsqueda** de animes por nombre con resultados en tiempo real.
- **Catálogo** filtrable por género, demografía, temporada, tipo, estado y año.
- **Detalle de anime**: sinopsis, temporada, estado, géneros y listado de episodios.
- **Reproductor basado en libmpv** en un hilo dedicado:
  - Reanuda donde lo dejaste (progreso guardado cada 10 s).
  - Velocidad, volumen, silencio, pantalla completa y subtítulos.
  - Cache de demuxer propio para suavizar fluctuaciones de red.
- **Historial** de reproducción ("continuar viendo").
- **Favoritos**.
- **Smart Buffer**: precarga en segundo plano de episodios futuros en segmentos
  HLS para reducir microcortes y el tiempo de espera al cambiar de episodio.
  - Límites configurables: nº de episodios, porcentaje, caché en disco (MB),
    ancho de banda (Mbps), umbral de arranque y umbral de CPU.
- **Persistencia** en SQLite (WAL) con migraciones.
- Proveedor **JKAnime** mediante scraping, con trait `Provider` extensible a otros.

## Stack

| Capa | Tecnología |
|---|---|
| Frontend | React 18, TypeScript, Vite, TailwindCSS, Zustand |
| Backend | Rust, Tauri 2, tokio |
| Red | reqwest (HTTP/2, cookies, retry con backoff) |
| Base de datos | SQLite + sqlx (migraciones) |
| Reproducción | libmpv (`mpv-rs`) |
| Scraping | scraper + parsing HLS propio |

## Arquitectura

```
Frontend (React + Zustand)
   │  invoke() / player://* , buffer://* (eventos)
   ▼
commands.rs ──► services
                   ├── anime.rs      búsqueda, catálogo, detalle, resolución de video
                   ├── player.rs     hilo libmpv + guardado de progreso
                   ├── buffer.rs     Smart Buffer (precarga HLS en segundo plano)
                   ├── history.rs    historial de reproducción
                   └── favorite.rs   favoritos
                   ├── settings.rs   configuración persistente del buffer
provider ──► jkanime.rs / jkanime_parse.rs
infra
   ├── db.rs / repos.rs      SQLite (sqlx)
   ├── http.rs               HttpClient (reqwest)
   ├── cache.rs              cachés TTL (mem/disk)
   ├── hls.rs                parsing y descarga de segmentos HLS
   ├── segcache.rs           caché de segmentos en disco
   └── segserver.rs          servidor local de segmentos
```

## Requisitos

- Node.js ≥ 18
- Rust (toolchain estable) y dependencias de sistema de [Tauri](https://v2.tauri.app/start/prerequisites/)
  (`libwebkit2gtk-4.1`, `libgtk-3`, etc.)
- `libmpv` (paquete de desarrollo)

## Desarrollo

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

Los instaladores se generan en `src-tauri/target/release/bundle/`.

## Documentación técnica

- [docs/buffer/fase1-analisis.md](docs/buffer/fase1-analisis.md) — análisis del Smart Buffer
- [docs/buffer/fase2-diseno.md](docs/buffer/fase2-diseno.md) — diseño del Smart Buffer
- [docs/buffer/fase3-plan.md](docs/buffer/fase3-plan.md) — plan de implementación

## Licencia

Este proyecto es solo una interfaz para interactuar con sitios de streaming.
El contenido reproducido pertenece a sus respectivos propietarios.
