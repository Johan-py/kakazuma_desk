use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;

mod commands;
mod domain;
mod error;
mod infra;
mod provider;
mod services;
mod settings;
mod state;
use crate::error::AppError;
use crate::infra::cache::{CacheRegistry, DiskCache};
use crate::infra::db::Db;
use crate::infra::http::{HttpClient, HttpConfig};
use crate::infra::segcache::SegmentCache;
use crate::provider::{JKAnimeProvider, Provider};
use crate::services::{AnimeService, BufferService, FavoriteService, HistoryService, PlayerCommand, PlayerService};
use crate::settings::SettingsService;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kakasuma_desktop=info,tauri=warn".into()),
        )
        .compact()
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle();
            let app_data = app_handle
                .path()
                .app_data_dir()
                .map_err(|e| AppError::Config(format!("directorio de datos: {e}")))?;
            std::fs::create_dir_all(&app_data)
                .map_err(|e| AppError::Config(format!("crear {app_data:?}: {e}")))?;

            let db_path = app_data.join("kakasuma.db");

            let db = tauri::async_runtime::block_on(async {
                let db = Db::connect(&db_path).await?;
                db.migrate().await?;
                Ok::<_, AppError>(db)
            })?;
            let pool = db.pool().clone();

            let http = HttpClient::new(HttpConfig::default())?;
            let provider: std::sync::Arc<dyn Provider> =
                std::sync::Arc::new(JKAnimeProvider::new(http.clone()));

            let disk = DiskCache::new(&app_data, "cache", Duration::from_secs(15 * 60))?;
            let mut registry = CacheRegistry::new();
            registry.register("genres", Duration::from_secs(24 * 3600));

            let settings = std::sync::Arc::new(SettingsService::new(pool.clone()));
            tauri::async_runtime::block_on(async { settings.load().await })?;

            let anime = std::sync::Arc::new(AnimeService::new(
                provider.clone(),
                http.clone(),
                disk,
                pool.clone(),
            ));
            let history = HistoryService::new(pool.clone());
            let favorites = FavoriteService::new(pool.clone());

            let buffer_cache = std::sync::Arc::new(SegmentCache::new(&app_data, "buffer")?);
            let cfg = settings.config();
            buffer_cache.set_limit(cfg.buffer_cache_limit_mb);

            let rt = tauri::async_runtime::handle();
            let player = PlayerService::spawn(app_handle.clone(), pool.clone(), rt.clone());
            let player_state = player.state_handle();
            let buffer = BufferService::spawn(
                app_handle.clone(),
                anime.clone(),
                pool.clone(),
                http,
                buffer_cache,
                settings.clone(),
                player_state,
                rt,
            );

            app_handle.manage(AppState {
                anime,
                history,
                favorites,
                player: Mutex::new(player),
                settings,
                buffer,
            });
            let _ = &registry;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_anime,
            commands::get_anime_detail,
            commands::get_catalog,
            commands::get_tags,
            commands::get_recent,
            commands::get_recommended,
            commands::resolve_video,
            commands::list_favorites,
            commands::add_favorite,
            commands::remove_favorite,
            commands::is_favorite,
            commands::continue_watching,
            commands::save_progress,
            commands::clear_history,
            commands::play_episode,
            commands::player_pause,
            commands::player_resume,
            commands::player_toggle_pause,
            commands::player_seek,
            commands::player_set_speed,
            commands::player_set_volume,
            commands::player_toggle_mute,
            commands::player_fullscreen,
            commands::player_stop,
            commands::player_get_state,
            commands::buffer_get_config,
            commands::buffer_set_config,
            commands::buffer_get_status,
            commands::buffer_clear_cache,
            commands::buffer_pause,
        ])
        .build(tauri::generate_context!())
        .expect("error al construir la aplicación Kakasuma")
        .run(|app_handle, event| {
            use tauri::RunEvent;
            match event {
                RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        if let Ok(player) = state.player.lock() {
                            player.send(PlayerCommand::Stop);
                        }
                        state.buffer.shutdown();
                    }
                }
                _ => {}
            }
        });
}
