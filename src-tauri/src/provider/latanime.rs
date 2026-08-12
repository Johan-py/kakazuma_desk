use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tracing::{debug, warn};

use crate::domain::{Anime, CatalogFilter, CatalogPage, Episode, Tag, VideoSource};
use crate::error::{AppError, AppResult};
use crate::infra::http::HttpClient;
use crate::provider::latanime_parse::{
    catalog_url, dsvplay_pass_md5_path, hexload_embed_id, mirror_of, parse_anime_detail,
    parse_catalog, parse_episodes, parse_genres, parse_recent, parse_recommended, parse_search,
    player_embed_urls, MIRROR_DSVPLAY, MIRROR_HEXLOAD,
};
use crate::provider::Provider;

/// Proveedor alternativo: Latanime (latanime.org).
///
/// Scraping de búsqueda, catálogo, detalle, episodios y video (espejos
/// dsvplay/DoodStream y hexload, ambos sirven MP4 directo sin captcha).
pub struct LatanimeProvider {
    http: HttpClient,
    home_cache: Mutex<Option<(Instant, String)>>,
}

impl LatanimeProvider {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
            home_cache: Mutex::new(None),
        }
    }

    /// Home cacheada brevemente (reciente + recomendados).
    async fn home(&self) -> AppResult<String> {
        if let Ok(cache) = self.home_cache.lock() {
            if let Some((at, html)) = cache.as_ref() {
                if at.elapsed() < Duration::from_secs(120) {
                    return Ok(html.clone());
                }
            }
        }
        let html = self.http.get_text("https://latanime.org/").await?;
        if let Ok(mut cache) = self.home_cache.lock() {
            *cache = Some((Instant::now(), html.clone()));
        }
        Ok(html)
    }

    /// Resuelve la URL del stream a partir de un embed de espejo.
    async fn resolve_embed(&self, embed: &str) -> Option<String> {
        match mirror_of(embed) {
            MIRROR_DSVPLAY => self.resolve_dsvplay(embed).await,
            MIRROR_HEXLOAD => self.resolve_hexload(embed).await,
            _ => {
                debug!(embed, "espejo sin extractor, usando embed directo");
                Some(embed.to_string())
            }
        }
    }

    async fn resolve_dsvplay(&self, embed: &str) -> Option<String> {
        let embed_html = self.http.get_text(embed).await.ok()?;
        let path = dsvplay_pass_md5_path(&embed_html)?;
        let path = path.trim_start_matches('/');
        let md5_url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("https://dsvplay.com/{path}")
        };
        let body = self.http.get_text(&md5_url).await.ok()?;
        let base = body.trim().trim_matches('"').to_string();
        if base.is_empty() {
            return None;
        }

        let token = path
            .rsplit('/')
            .next()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let suffix = makeplay_suffix();
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        Some(format!("{base}{suffix}?token={token}&expiry={expiry}"))
    }

    async fn resolve_hexload(&self, embed: &str) -> Option<String> {
        let id = hexload_embed_id(embed)?;
        let body = self
            .http
            .post_form(
                "https://hexload.com/download",
                &[
                    ("op", "download3".into()),
                    ("id", id.clone()),
                    ("ajax", "1".into()),
                    ("method_free", "1".into()),
                ],
                &[
                    ("X-Requested-With", "XMLHttpRequest"),
                    ("Referer", embed),
                ],
            )
            .await
            .ok()?;

        let parsed: HexloadResponse = serde_json::from_str(&body).ok()?;
        let url = parsed.result.map(|r| r.url).unwrap_or_default();
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct HexloadResponse {
    result: Option<HexloadResult>,
}

#[derive(Debug, serde::Deserialize)]
struct HexloadResult {
    #[serde(default)]
    url: String,
}

/// Marca aleatoria estilo `makePlay()` que exige DoodStream en la URL final.
fn makeplay_suffix() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = now as u64;
    const CHARSET: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut s = String::with_capacity(10);
    for _ in 0..10 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        s.push(CHARSET[(x as usize) % CHARSET.len()] as char);
    }
    s
}

#[async_trait]
impl Provider for LatanimeProvider {
    fn name(&self) -> &'static str {
        "Latanime"
    }

    async fn search(&self, query: &str) -> AppResult<Vec<Anime>> {
        let encoded = utf8_percent_encode(query.trim(), NON_ALPHANUMERIC).to_string();
        let url = format!("https://latanime.org/buscar?q={encoded}");
        let html = self.http.get_text(&url).await?;
        let items = parse_search(&html)?;
        debug!(query, count = items.len(), "búsqueda completada");
        Ok(items)
    }

    async fn get_anime(&self, slug: &str) -> AppResult<Anime> {
        let url = format!("https://latanime.org/anime/{slug}/");
        let html = self.http.get_text(&url).await?;
        parse_anime_detail(&html, slug)
    }

    async fn get_episodes(&self, slug: &str) -> AppResult<Vec<Episode>> {
        let url = format!("https://latanime.org/anime/{slug}/");
        let html = self.http.get_text(&url).await?;
        let episodes = parse_episodes(&html)?;
        debug!(slug, count = episodes.len(), "episodios obtenidos");
        Ok(episodes)
    }

    async fn resolve_video(&self, slug: &str, number: i32) -> AppResult<VideoSource> {
        let ep_url = format!("https://latanime.org/ver/{slug}-episodio-{number}/");
        let html = self.http.get_text(&ep_url).await?;

        let embeds = player_embed_urls(&html);
        if embeds.is_empty() {
            return Err(AppError::Provider(format!(
                "no se encontraron mirrors de video en {ep_url}"
            )));
        }

        for embed in embeds {
            if let Some(url) = self.resolve_embed(&embed).await {
                return Ok(VideoSource {
                    url,
                    quality: Some("MP4".into()),
                    subtitles: Vec::new(),
                });
            }
            warn!(embed, "espejo fallido en latanime");
        }
        Err(AppError::Provider(format!(
            "no se pudo resolver el video desde {ep_url}"
        )))
    }

    async fn catalog(&self, filter: &CatalogFilter, page: u32) -> AppResult<CatalogPage> {
        let url = catalog_url(filter, page);
        let html = self.http.get_text(&url).await?;
        parse_catalog(&html, page)
    }

    async fn recent(&self) -> AppResult<Vec<Anime>> {
        let home = self.home().await?;
        parse_recent(&home)
    }

    async fn recommended(&self) -> AppResult<Vec<Anime>> {
        let home = self.home().await?;
        parse_recommended(&home)
    }

    async fn genres(&self) -> AppResult<Vec<Tag>> {
        let dir = self.http.get_text("https://latanime.org/animes").await?;
        parse_genres(&dir)
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::infra::http::HttpConfig;

    async fn provider() -> LatanimeProvider {
        let http = HttpClient::new(HttpConfig::default()).unwrap();
        LatanimeProvider::new(http)
    }

    #[tokio::test]
    #[ignore = "requiere red"]
    async fn search_works() {
        let p = provider().await;
        let items = p.search("naruto").await.unwrap();
        assert!(!items.is_empty());
        let a = &items[0];
        assert!(!a.slug.is_empty());
        assert!(!a.name.is_empty());
    }

    #[tokio::test]
    #[ignore = "requiere red"]
    async fn detail_and_episodes_work() {
        let p = provider().await;
        let slug = "oshi-no-ko";
        let anime = p.get_anime(slug).await.unwrap();
        assert_eq!(anime.slug, slug);
        assert!(!anime.name.is_empty());
        let episodes = p.get_episodes(slug).await.unwrap();
        assert!(episodes.len() > 0);
    }

    #[tokio::test]
    #[ignore = "requiere red"]
    async fn resolve_video_works() {
        let p = provider().await;
        let source = p.resolve_video("oshi-no-ko", 1).await.unwrap();
        assert!(!source.url.is_empty());
    }

    #[tokio::test]
    #[ignore = "requiere red"]
    async fn home_and_genres_work() {
        let p = provider().await;
        let recent = p.recent().await.unwrap();
        assert!(!recent.is_empty());
        let recommended = p.recommended().await.unwrap();
        assert!(!recommended.is_empty());
        let genres = p.genres().await.unwrap();
        assert!(genres.len() > 10);
    }

    #[tokio::test]
    #[ignore = "requiere red"]
    async fn catalog_works() {
        use crate::domain::CatalogFilter;
        let p = provider().await;
        let page = p.catalog(&CatalogFilter::default(), 1).await.unwrap();
        assert!(!page.items.is_empty());
        assert!(page.last_page >= 1);
    }
}