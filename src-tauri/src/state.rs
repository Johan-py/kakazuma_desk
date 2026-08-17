use std::sync::{Arc, Mutex};

use crate::provider::ProviderRegistry;
use crate::services::{AnimeService, BufferService, FavoriteService, HistoryService, PlayerState};
use crate::settings::SettingsService;

/// Estado global de la aplicación compartido con los comandos Tauri.
pub struct AppState {
    pub anime: Arc<AnimeService>,
    pub history: HistoryService,
    pub favorites: FavoriteService,
    pub player_state: Arc<Mutex<PlayerState>>,
    pub settings: Arc<SettingsService>,
    pub buffer: BufferService,
    pub provider: Arc<ProviderRegistry>,
}
