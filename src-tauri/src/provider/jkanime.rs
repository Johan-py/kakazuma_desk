use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tracing::{debug, warn};

use crate::domain::{Anime, CatalogFilter, CatalogPage, Episode, Tag, VideoSource};
use crate::error::{AppError, AppResult};
use crate::infra::http::{resolve_url, HttpClient};
use crate::provider::jkanime_parse::{
    catalog_url, extract_page_info, extract_video_from_player, find_player_url, parse_anime_detail,
    parse_catalog, parse_episodes_response, parse_genres, parse_recent, parse_recommended,
    parse_search, EpisodesResponse, BASE_URL,
};
use crate::provider::Provider;

#[derive(Debug, Clone)]
struct PageInfo {
    id: String,
    csrf: String,
    fetched: Instant,
}

/// Proveedor oficial inicial: JKAnime (jkanime.net).
///
/// Todo el scraping vive aquí. Otros proveedores (AnimeFLV, TioAnime,
/// Crunchyroll...) deben implementar el mismo trait `Provider` sin tocar la
/// lógica de negocio existente.
pub struct JKAnimeProvider {
    http: HttpClient,
    page_cache: Mutex<HashMap<String, PageInfo>>,
}

impl JKAnimeProvider {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
            page_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Id numérico y token CSRF de la página de un anime (cacheado 10 min).
    async fn page_info(&self, slug: &str) -> AppResult<PageInfo> {
        if let Ok(cache) = self.page_cache.lock() {
            if let Some(info) = cache.get(slug) {
                if info.fetched.elapsed() < Duration::from_secs(600) {
                    return Ok(info.clone());
                }
            }
        }

        let url = format!("{BASE_URL}/{slug}/");
        let html = self.http.get_text(&url).await?;
        let (id, csrf) = extract_page_info(&html, slug)?;
        let info = PageInfo { id, csrf, fetched: Instant::now() };

        if let Ok(mut cache) = self.page_cache.lock() {
            cache.insert(slug.to_string(), info.clone());
        }
        Ok(info)
    }
}

#[async_trait]
impl Provider for JKAnimeProvider {
    fn name(&self) -> &'static str {
        "JKAnime"
    }

    async fn search(&self, query: &str) -> AppResult<Vec<Anime>> {
        let encoded = utf8_percent_encode(query, NON_ALPHANUMERIC)
            .to_string()
            .replace("%20", "_");
        let url = format!("{BASE_URL}/buscar/{encoded}/");
        let html = self.http.get_text(&url).await?;
        let items = parse_search(&html)?;
        debug!(query, count = items.len(), "búsqueda completada");
        Ok(items)
    }

    async fn get_anime(&self, slug: &str) -> AppResult<Anime> {
        let url = format!("{BASE_URL}/{slug}/");
        let html = self.http.get_text(&url).await?;
        parse_anime_detail(&html, slug)
    }

    async fn get_episodes(&self, slug: &str) -> AppResult<Vec<Episode>> {
        let info = self.page_info(slug).await?;
        let api = format!("{BASE_URL}/ajax/episodes/{}/", info.id);
        let token = info.csrf.clone();

        let first = self
            .http
            .post_form(
                &api,
                &[("_token", token.clone())],
                &[("X-Requested-With", "XMLHttpRequest"), ("X-CSRF-TOKEN", &token)],
            )
            .await?;
        let parsed: EpisodesResponse = serde_json::from_str(&first)
            .map_err(|e| AppError::Provider(format!("JSON de episodios: {e}")))?;

        let mut episodes = parse_episodes_response(parsed.clone());

        if parsed.last_page > 1 {
            let pages: Vec<u32> = (2..=parsed.last_page).collect();
            let mut set = tokio::task::JoinSet::new();
            for p in pages {
                let api = api.clone();
                let token = token.clone();
                let http = self.http.clone();
                set.spawn(async move {
                    let url = format!("{api}?p={p}");
                    let body = http
                        .post_form(
                            &url,
                            &[("_token", token.clone())],
                            &[("X-Requested-With", "XMLHttpRequest"), ("X-CSRF-TOKEN", &token)],
                        )
                        .await?;
                    let r: EpisodesResponse = serde_json::from_str(&body)
                        .map_err(|e| AppError::Provider(format!("JSON de episodios: {e}")))?;
                    AppResult::Ok(r)
                });
                if set.len() >= 6 {
                    drain_join_set(&mut set, &mut episodes).await;
                }
            }
            drain_join_set(&mut set, &mut episodes).await;
        }

        episodes.sort_by_key(|e| e.number);
        episodes.dedup_by_key(|e| e.number);
        debug!(slug, count = episodes.len(), "episodios obtenidos");
        Ok(episodes)
    }

    async fn resolve_video(&self, slug: &str, number: i32) -> AppResult<VideoSource> {
        let ep_url = format!("{BASE_URL}/{slug}/{number}/");
        let html = self.http.get_text(&ep_url).await?;

        let candidate = find_player_url(&html)
            .ok_or_else(|| AppError::Provider(format!("no se encontró reproductor en {ep_url}")))?;

        let player_url = resolve_url(&candidate, BASE_URL);
        let player_html = self.http.get_text(&player_url).await?;

        let video_url = extract_video_from_player(&player_html)
            .ok_or_else(|| AppError::Provider(format!("no se pudo extraer el video desde {player_url}")))?;

        Ok(VideoSource {
            url: video_url,
            quality: Some("HLS".into()),
            subtitles: Vec::new(),
        })
    }

    async fn catalog(&self, filter: &CatalogFilter, page: u32) -> AppResult<CatalogPage> {
        let url = catalog_url(filter, page);
        let html = self.http.get_text(&url).await?;
        parse_catalog(&html)
    }

    async fn recent(&self) -> AppResult<Vec<Anime>> {
        let home = self.http.get_text(BASE_URL).await?;
        parse_recent(&home)
    }

    async fn recommended(&self) -> AppResult<Vec<Anime>> {
        let home = self.http.get_text(BASE_URL).await?;
        parse_recommended(&home)
    }

    async fn genres(&self) -> AppResult<Vec<Tag>> {
        let dir = self.http.get_text(&format!("{BASE_URL}/directorio")).await?;
        parse_genres(&dir)
    }
}

async fn drain_join_set(
    set: &mut tokio::task::JoinSet<AppResult<EpisodesResponse>>,
    episodes: &mut Vec<Episode>,
) {
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(r)) => episodes.extend(parse_episodes_response(r)),
            Ok(Err(e)) => warn!(error = %e, "fallo al obtener página de episodios"),
            Err(e) => warn!(error = %e, "tarea de episodios abortada"),
        }
    }
}
