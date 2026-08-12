use std::sync::{Arc, Mutex};

use crate::provider::ProviderRegistry;
use crate::services::{AnimeService, BufferService, FavoriteService, HistoryService, PlayerService};
use crate::settings::SettingsService;

/// Estado global de la aplicación compartido con los comandos Tauri.
pub struct AppState {
    pub anime: Arc<AnimeService>,
    pub history: HistoryService,
    pub favorites: FavoriteService,
    pub player: Mutex<PlayerService>,
    pub settings: Arc<SettingsService>,
    pub buffer: BufferService,
    pub provider: Arc<ProviderRegistry>,
}