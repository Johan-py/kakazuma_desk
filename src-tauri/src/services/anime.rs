use std::sync::Arc;

use sqlx::SqlitePool;
use tracing::debug;

use crate::domain::{Anime, AnimeDetail, CatalogFilter, CatalogPage, Episode, Tag, VideoSource};
use crate::error::AppResult;
use crate::infra::cache::{DiskCache, TtlCache};
use crate::infra::http::HttpClient;
use crate::infra::repos::{AnimeRepo, EpisodeRepo, TagRepo};
use crate::provider::{Provider, ProviderRegistry};

/// Caché en memoria: LRU con TTL de 15 minutos.
const MEM_CAPACITY: usize = 256;
const MEM_TTL_SECS: u64 = 15 * 60;

/// Servicio de animes. Conecta proveedor + caché + base de datos.
pub struct AnimeService {
    providers: Arc<ProviderRegistry>,
    _http: HttpClient,
    mem: TtlCache<Vec<Anime>>,
    tags_mem: TtlCache<Vec<Tag>>,
    detail_mem: TtlCache<AnimeDetail>,
    disk: DiskCache,
    pool: SqlitePool,
}

impl AnimeService {
    pub fn new(
        providers: Arc<ProviderRegistry>,
        http: HttpClient,
        disk: DiskCache,
        pool: SqlitePool,
    ) -> Self {
        Self {
            providers,
            _http: http,
            mem: TtlCache::new(MEM_CAPACITY, std::time::Duration::from_secs(MEM_TTL_SECS)),
            tags_mem: TtlCache::new(
                MEM_CAPACITY,
                std::time::Duration::from_secs(MEM_TTL_SECS),
            ),
            detail_mem: TtlCache::new(
                MEM_CAPACITY,
                std::time::Duration::from_secs(MEM_TTL_SECS),
            ),
            disk,
            pool,
        }
    }

    /// Proveedor activo según la configuración del usuario.
    fn provider(&self) -> Arc<dyn Provider> {
        self.providers.default()
    }

    /// Clave con namespaces: el proveedor activo evita servir datos de otro.
    fn key(&self, kind: &str, slug: &str) -> String {
        format!("{}:{}:{}", self.providers.default_key(), kind, slug)
    }

    pub async fn search(&self, query: &str) -> AppResult<Vec<Anime>> {
        let key = self.key("search", &query.trim().to_lowercase());
        if let Some(cached) = self.mem.get(&key) {
            return Ok(cached);
        }
        if let Some(cached) = self.disk.get::<Vec<Anime>>(&key) {
            self.mem.put(key.clone(), cached.clone());
            return Ok(cached);
        }

        let items = self.provider().search(query).await?;
        self.mem.put(key.clone(), items.clone());
        self.disk.put(&key, &items)?;
        debug!(query, count = items.len(), "búsqueda cacheada");
        Ok(items)
    }

    pub async fn get_anime_detail(&self, slug: &str) -> AppResult<AnimeDetail> {
        let key = self.key("detail", &slug.to_lowercase());
        if let Some(cached) = self.detail_mem.get(&key) {
            return Ok(cached);
        }
        if let Some(cached) = self.disk.get::<AnimeDetail>(&key) {
            self.detail_mem.put(key.clone(), cached.clone());
            return Ok(cached);
        }

        // Fetch desde el proveedor y persistir en la base local.
        let provider = self.provider();
        let anime = provider.get_anime(slug).await?;
        let episodes = provider.get_episodes(slug).await?;

        let db_id = AnimeRepo::upsert(&self.pool, &anime).await?;

        let persisted_episodes: Vec<Episode> = episodes
            .iter()
            .map(|e| Episode {
                anime_id: db_id,
                ..e.clone()
            })
            .collect();
        EpisodeRepo::upsert_many(&self.pool, db_id, &persisted_episodes).await?;

        if !anime.genres.is_empty() {
            TagRepo::set_for_anime(&self.pool, db_id, &anime.genres).await?;
        }

        let mut final_anime = anime;
        final_anime.id = db_id;
        final_anime.total_episodes = Some(persisted_episodes.len() as i32);

        let tags: Vec<Tag> = final_anime
            .genres
            .iter()
            .map(|g| Tag { id: 0, name: g.clone(), description: None })
            .collect();

        let detail = AnimeDetail {
            anime: final_anime,
            episodes: persisted_episodes,
            tags,
        };
        self.detail_mem.put(key.clone(), detail.clone());
        self.disk.put(&key, &detail)?;
        debug!(slug, episodes = detail.episodes.len(), "detalle cacheado");
        Ok(detail)
    }

    pub async fn catalog(&self, filter: &CatalogFilter, page: u32) -> AppResult<CatalogPage> {
        let key = format!("{}:catalog:{filter:?}:{page}", self.providers.default_key());
        if let Some(cached) = self.disk.get::<CatalogPage>(&key) {
            return Ok(cached);
        }

        let result = self.provider().catalog(filter, page).await?;
        self.disk.put(&key, &result)?;
        Ok(result)
    }

    pub async fn recent(&self) -> AppResult<Vec<Anime>> {
        let key = self.key("recent", "");
        if let Some(cached) = self.mem.get(&key) {
            return Ok(cached);
        }
        if let Some(cached) = self.disk.get::<Vec<Anime>>(&key) {
            self.mem.put(key.clone(), cached.clone());
            return Ok(cached);
        }

        let items = self.provider().recent().await?;
        self.mem.put(key.clone(), items.clone());
        self.disk.put(&key, &items)?;
        Ok(items)
    }

    pub async fn recommended(&self) -> AppResult<Vec<Anime>> {
        let key = self.key("recommended", "");
        if let Some(cached) = self.mem.get(&key) {
            return Ok(cached);
        }
        if let Some(cached) = self.disk.get::<Vec<Anime>>(&key) {
            self.mem.put(key.clone(), cached.clone());
            return Ok(cached);
        }

        let items = self.provider().recommended().await?;
        self.mem.put(key.clone(), items.clone());
        self.disk.put(&key, &items)?;
        Ok(items)
    }

    pub async fn genres(&self) -> AppResult<Vec<Tag>> {
        let key = self.key("genres", "");
        if let Some(cached) = self.tags_mem.get(&key) {
            return Ok(cached);
        }
        if let Some(cached) = self.disk.get::<Vec<Tag>>(&key) {
            self.tags_mem.put(key.clone(), cached.clone());
            return Ok(cached);
        }

        let tags = self.provider().genres().await?;
        self.tags_mem.put(key.clone(), tags.clone());
        self.disk.put(&key, &tags)?;
        Ok(tags)
    }

    pub async fn resolve_video(&self, slug: &str, number: i32) -> AppResult<VideoSource> {
        // Sin caché: la URL del stream expira.
        self.provider().resolve_video(slug, number).await
    }
}
