use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use async_trait::async_trait;
use serde::Serialize;

use crate::domain::{Anime, CatalogFilter, CatalogPage, Episode, Tag, VideoSource};
use crate::error::AppResult;
use crate::infra::http::HttpClient;

pub mod jkanime;
pub mod jkanime_parse;
pub mod latanime;
pub mod latanime_parse;

pub use jkanime::JKAnimeProvider;
pub use latanime::LatanimeProvider;

/// Identificador estable de cada proveedor (clave persistida en settings).
pub const PROVIDER_JKANIME: &str = "jkanime";
pub const PROVIDER_LATANIME: &str = "latanime";
pub const DEFAULT_PROVIDER: &str = PROVIDER_JKANIME;

/// Contrato común para cualquier proveedor de contenido.
///
/// Toda la lógica de scraping debe quedar encapsulada en las implementaciones
/// de este trait y nunca mezclarse con la UI o la capa de servicios.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Nombre visible (p. ej. "JKAnime").
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

/// Descriptor que se expone al frontend para el selector de fuente.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderDescriptor {
    pub key: &'static str,
    pub name: &'static str,
}

/// Registro de proveedores disponibles con la fuente por defecto seleccionable.
///
/// Los servicios (y el buffer) acceden siempre a través de [`ProviderRegistry::default`],
/// de modo que al cambiar la fuente desde la UI todo el scraping fluye al
/// proveedor activo sin reconfigurar nada más.
pub struct ProviderRegistry {
    providers: HashMap<&'static str, Arc<dyn Provider>>,
    default: RwLock<String>,
}

impl ProviderRegistry {
    pub fn new(http: HttpClient) -> Self {
        let mut providers: HashMap<&'static str, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            PROVIDER_JKANIME,
            Arc::new(JKAnimeProvider::new(http.clone())) as Arc<dyn Provider>,
        );
        providers.insert(
            PROVIDER_LATANIME,
            Arc::new(LatanimeProvider::new(http.clone())) as Arc<dyn Provider>,
        );
        Self {
            providers,
            default: RwLock::new(DEFAULT_PROVIDER.to_string()),
        }
    }

    /// Proveedor activo (el que usa la UI y el buffer).
    pub fn default(&self) -> Arc<dyn Provider> {
        let key = self.default_key();
        self.providers
            .get(key.as_str())
            .cloned()
            .unwrap_or_else(|| self.providers[DEFAULT_PROVIDER].clone())
    }

    /// Clave del proveedor activo.
    pub fn default_key(&self) -> String {
        self.default
            .read()
            .map(|k| k.clone())
            .unwrap_or_else(|_| DEFAULT_PROVIDER.to_string())
    }

    /// Cambia el proveedor activo. Devuelve `false` si la clave no existe.
    pub fn set_default(&self, key: &str) -> bool {
        if !self.providers.contains_key(key) {
            return false;
        }
        if let Ok(mut d) = self.default.write() {
            *d = key.to_string();
            true
        } else {
            false
        }
    }

    /// Descriptores de todos los proveedores disponibles.
    pub fn list(&self) -> Vec<ProviderDescriptor> {
        self.providers
            .iter()
            .map(|(key, p)| ProviderDescriptor {
                key,
                name: p.name(),
            })
            .collect()
    }

    /// Indica si una clave de proveedor existe en el registro.
    pub fn valid_key(key: &str) -> bool {
        matches!(key, PROVIDER_JKANIME | PROVIDER_LATANIME)
    }
}