use async_trait::async_trait;

use crate::domain::{Anime, CatalogFilter, CatalogPage, Episode, Tag, VideoSource};
use crate::error::AppResult;

pub mod jkanime;
pub mod jkanime_parse;

pub use jkanime::JKAnimeProvider;

/// Contrato común para cualquier proveedor de contenido.
///
/// Toda la lógica de scraping debe quedar encapsulada en las implementaciones
/// de este trait y nunca mezclarse con la UI o la capa de servicios.
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn search(&self, query: &str) -> AppResult<Vec<Anime>>;

    async fn get_anime(&self, slug: &str) -> AppResult<Anime>;

    async fn get_episodes(&self, slug: &str) -> AppResult<Vec<Episode>>;

    async fn resolve_video(&self, slug: &str, number: i32) -> AppResult<VideoSource>;

    async fn catalog(&self, filter: &CatalogFilter, page: u32) -> AppResult<CatalogPage>;

    async fn recent(&self) -> AppResult<Vec<Anime>>;

    async fn recommended(&self) -> AppResult<Vec<Anime>>;

    async fn genres(&self) -> AppResult<Vec<Tag>>;
}
