use tauri::State;

use crate::domain::{
    Anime, AnimeDetail, CatalogFilter, CatalogPage, FavoriteEntry, ProviderInfo, ProviderOption,
    Tag, VideoSource, WatchHistoryEntry,
};
use crate::error::AppError;
use crate::services::{BufferStatus, PlayerCommand};
use crate::settings::BufferConfig;
use crate::state::AppState;

/// Busca animes en el proveedor.
#[tauri::command]
pub async fn search_anime(state: State<'_, AppState>, query: String) -> Result<Vec<Anime>, AppError> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    state.anime.search(&q).await
}

/// Fuente por defecto y opciones disponibles.
#[tauri::command]
pub fn get_provider(state: State<'_, AppState>) -> ProviderInfo {
    provider_info(&state)
}

/// Cambia la fuente por defecto (persistida) y resetea el proveedor activo.
#[tauri::command]
pub async fn set_provider(
    state: State<'_, AppState>,
    provider: String,
) -> Result<ProviderInfo, AppError> {
    if !state.provider.set_default(&provider) {
        return Err(AppError::Config(format!(
            "proveedor desconocido: {provider}"
        )));
    }
    state.settings.set_provider_key(&provider).await?;
    Ok(provider_info(&state))
}

fn provider_info(state: &AppState) -> ProviderInfo {
    let current = state.settings.provider_key();
    let available: Vec<ProviderOption> = state
        .provider
        .list()
        .into_iter()
        .map(|d| ProviderOption {
            key: d.key.to_string(),
            name: d.name.to_string(),
        })
        .collect();
    ProviderInfo { current, available }
}

/// Detalle completo de un anime (info + episodios), persistido localmente.
#[tauri::command]
pub async fn get_anime_detail(state: State<'_, AppState>, slug: String) -> Result<AnimeDetail, AppError> {
    state.anime.get_anime_detail(&slug).await
}

/// Catálogo con filtros paginado.
#[tauri::command]
pub async fn get_catalog(
    state: State<'_, AppState>,
    filter: CatalogFilter,
    page: u32,
) -> Result<CatalogPage, AppError> {
    state.anime.catalog(&filter, page.max(1)).await
}

/// Etiquetas (géneros) disponibles.
#[tauri::command]
pub async fn get_tags(state: State<'_, AppState>) -> Result<Vec<Tag>, AppError> {
    state.anime.genres().await
}

/// Animes recientes para la home.
#[tauri::command]
pub async fn get_recent(state: State<'_, AppState>) -> Result<Vec<Anime>, AppError> {
    state.anime.recent().await
}

/// Animes recomendados (top) para la home.
#[tauri::command]
pub async fn get_recommended(state: State<'_, AppState>) -> Result<Vec<Anime>, AppError> {
    state.anime.recommended().await
}

/// Resuelve la URL directa del video de un episodio.
#[tauri::command]
pub async fn resolve_video(
    state: State<'_, AppState>,
    slug: String,
    number: i32,
) -> Result<VideoSource, AppError> {
    state.anime.resolve_video(&slug, number).await
}

/// Favoritos.
#[tauri::command]
pub async fn list_favorites(state: State<'_, AppState>) -> Result<Vec<FavoriteEntry>, AppError> {
    state.favorites.list().await
}

#[tauri::command]
pub async fn add_favorite(state: State<'_, AppState>, slug: String) -> Result<bool, AppError> {
    state.favorites.add(&slug).await
}

#[tauri::command]
pub async fn remove_favorite(state: State<'_, AppState>, slug: String) -> Result<(), AppError> {
    state.favorites.remove(&slug).await
}

#[tauri::command]
pub async fn is_favorite(state: State<'_, AppState>, slug: String) -> Result<bool, AppError> {
    state.favorites.is_favorite(&slug).await
}

/// Historial / continuar viendo.
#[tauri::command]
pub async fn continue_watching(state: State<'_, AppState>) -> Result<Vec<WatchHistoryEntry>, AppError> {
    state.history.continue_watching().await
}

#[tauri::command]
pub async fn save_progress(
    state: State<'_, AppState>,
    slug: String,
    episode_number: Option<i32>,
    position: f64,
    duration: f64,
) -> Result<(), AppError> {
    state.history.save_progress(&slug, episode_number, position, duration).await
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AppState>) -> Result<(), AppError> {
    state.history.clear().await
}

/// Resuelve el video y arranca la reproducción en libmpv.
#[tauri::command]
pub async fn play_episode(
    state: State<'_, AppState>,
    slug: String,
    number: i32,
    title: String,
    start: f64,
) -> Result<VideoSource, AppError> {
    let source = state.anime.resolve_video(&slug, number).await?;
    let url = state.buffer.resolve_playback_url(&source, &slug, number).await;
    let player = state.player.lock().unwrap();
    player.play(&slug, number, &url, &title, start);
    Ok(source)
}

/// Smart Buffer: configuración.
#[tauri::command]
pub fn buffer_get_config(state: State<'_, AppState>) -> BufferConfig {
    state.settings.config()
}

#[tauri::command]
pub async fn buffer_set_config(
    state: State<'_, AppState>,
    config: BufferConfig,
) -> Result<BufferConfig, AppError> {
    let cfg = state.settings.set_config(config).await?;
    state.buffer.config_changed();
    Ok(cfg)
}

/// Smart Buffer: estado observable por la UI.
#[tauri::command]
pub fn buffer_get_status(state: State<'_, AppState>) -> BufferStatus {
    state.buffer.status()
}

/// Smart Buffer: limpia la caché y devuelve los bytes liberados.
#[tauri::command]
pub fn buffer_clear_cache(state: State<'_, AppState>) -> Result<u64, AppError> {
    let freed = state.buffer.cache().clear()?;
    state.buffer.clear_cache();
    Ok(freed)
}

/// Smart Buffer: pausa/resume manual de las descargas.
#[tauri::command]
pub fn buffer_pause(state: State<'_, AppState>, paused: bool) {
    state.buffer.set_manual_pause(paused);
}

/// Controles del reproductor.
#[tauri::command]
pub fn player_pause(state: State<'_, AppState>) {
    state.player.lock().unwrap().send(PlayerCommand::Pause);
}

#[tauri::command]
pub fn player_resume(state: State<'_, AppState>) {
    state.player.lock().unwrap().send(PlayerCommand::Resume);
}

#[tauri::command]
pub fn player_toggle_pause(state: State<'_, AppState>) {
    state.player.lock().unwrap().send(PlayerCommand::TogglePause);
}

#[tauri::command]
pub fn player_seek(state: State<'_, AppState>, position: f64) {
    state.player.lock().unwrap().send(PlayerCommand::Seek(position.max(0.0)));
}

#[tauri::command]
pub fn player_set_speed(state: State<'_, AppState>, speed: f64) {
    state.player.lock().unwrap().send(PlayerCommand::SetSpeed(speed));
}

#[tauri::command]
pub fn player_set_volume(state: State<'_, AppState>, volume: i64) {
    state.player.lock().unwrap().send(PlayerCommand::SetVolume(volume));
}

#[tauri::command]
pub fn player_toggle_mute(state: State<'_, AppState>) {
    state.player.lock().unwrap().send(PlayerCommand::ToggleMute);
}

#[tauri::command]
pub fn player_fullscreen(state: State<'_, AppState>, enabled: bool) {
    state.player.lock().unwrap().send(PlayerCommand::SetFullscreen(enabled));
}

#[tauri::command]
pub fn player_stop(state: State<'_, AppState>) {
    state.player.lock().unwrap().send(PlayerCommand::Stop);
}

#[tauri::command]
pub fn player_get_state(state: State<'_, AppState>) -> crate::services::PlayerState {
    state.player.lock().unwrap().get_state()
}
