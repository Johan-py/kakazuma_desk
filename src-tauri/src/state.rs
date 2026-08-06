use std::sync::Mutex;

use crate::services::{AnimeService, FavoriteService, HistoryService, PlayerService};

/// Estado global de la aplicación compartido con los comandos Tauri.
pub struct AppState {
    pub anime: AnimeService,
    pub history: HistoryService,
    pub favorites: FavoriteService,
    pub player: Mutex<PlayerService>,
}
