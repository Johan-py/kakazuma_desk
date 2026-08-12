use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Anime {
    /// id en la base de datos local. 0 si aún no está persistido.
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub synopsis: Option<String>,
    pub season: Option<String>,
    pub status: Option<String>,
    pub cover_image: Option<String>,
    pub total_episodes: Option<i32>,
    pub anime_type: Option<String>,
    pub url: String,
    pub genres: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Episode {
    /// id en la base de datos local. 0 si aún no está persistido.
    pub id: i64,
    pub anime_id: i64,
    pub number: i32,
    pub title: Option<String>,
    pub video_url: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subtitle {
    pub lang: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoSource {
    pub url: String,
    pub quality: Option<String>,
    pub subtitles: Vec<Subtitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnimeDetail {
    pub anime: Anime,
    pub episodes: Vec<Episode>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchHistoryEntry {
    pub id: i64,
    pub anime: Anime,
    pub episode: Option<Episode>,
    pub playback_position: f64,
    pub duration: f64,
    pub date_first_view: i64,
    pub date_last_view: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavoriteEntry {
    pub id: i64,
    pub anime: Anime,
    pub date_added: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogFilter {
    pub genero: Option<String>,
    pub demografia: Option<String>,
    pub temporada: Option<String>,
    pub tipo: Option<String>,
    pub estado: Option<String>,
    pub anio: Option<i32>,
    pub orden: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPage {
    pub items: Vec<Anime>,
    pub page: u32,
    pub total: u32,
    pub per_page: u32,
    pub last_page: u32,
}

/// Opción de fuente para el selector de la UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOption {
    pub key: String,
    pub name: String,
}

/// Proveedor configurado como fuente por defecto y opciones disponibles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub current: String,
    pub available: Vec<ProviderOption>,
}
