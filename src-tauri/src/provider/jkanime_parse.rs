use regex::Regex;
use scraper::{Html, Selector};
use serde::Deserialize;

use crate::domain::{Anime, CatalogFilter, CatalogPage, Episode, Tag};use crate::error::{AppError, AppResult};
use crate::infra::http::resolve_url;

pub const BASE_URL: &str = "https://jkanime.net";

pub fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn regex_capture(pattern: &str, hay: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    re.captures(hay).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

pub fn slug_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty() && !s.contains('.'))
        .map(|s| s.to_string())
}

fn parse_anime_item(el: scraper::ElementRef) -> Option<Anime> {
    let a_sel = Selector::parse("a").ok()?;
    let h5_sel = Selector::parse("h5 a").ok()?;
    let pic_sel = Selector::parse(".set-bg").ok()?;

    let href = el.select(&a_sel).next()?.value().attr("href")?.to_string();
    let name = el
        .select(&h5_sel)
        .next()?
        .text()
        .collect::<String>()
        .trim()
        .to_string();
    let cover = el
        .select(&pic_sel)
        .next()
        .and_then(|n| n.value().attr("data-setbg"))
        .map(|s| s.to_string());

    let slug = slug_from_url(&href)?;
    Some(Anime {
        id: 0,
        slug,
        name,
        synopsis: None,
        season: None,
        status: None,
        cover_image: cover,
        total_episodes: None,
        anime_type: None,
        url: resolve_url(&href, BASE_URL),
        genres: Vec::new(),
    })
}

/// Busca resultados en `/buscar/{query}/`.
pub fn parse_search(html: &str) -> AppResult<Vec<Anime>> {
    let doc = Html::parse_document(html);
    let selector =
        Selector::parse(".anime__item").map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    Ok(doc.select(&selector).filter_map(parse_anime_item).collect())
}

/// Extrae el id numérico y el token CSRF de la página de un anime.
pub fn extract_page_info(html: &str, slug: &str) -> AppResult<(String, String)> {
    let id = regex_capture(r"ajax/episodes/(\d+)", html)
        .or_else(|| regex_capture(r"ajax/search_episode/(\d+)", html))
        .ok_or_else(|| AppError::Provider(format!("no se encontró el id del anime en {slug}")))?;

    let csrf = regex_capture(r#"csrf-token" content="([^"]+)""#, html)
        .ok_or_else(|| AppError::Provider(format!("no se encontró token CSRF en {slug}")))?;

    Ok((id, csrf))
}

/// Detalles de un anime desde su página.
pub fn parse_anime_detail(html: &str, slug: &str) -> AppResult<Anime> {
    let doc = Html::parse_document(html);
    let sel = |s: &str| Selector::parse(s).expect("selector estático válido");

    let url = format!("{BASE_URL}/{slug}/");

    let name = doc
        .select(&sel(".anime_info h3"))
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .unwrap_or_else(|| slug.to_string());

    let synopsis = doc
        .select(&sel(".anime_info .scroll, .anime_info p"))
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .filter(|s| !s.is_empty());

    let cover_image = regex_capture(r#"property="og:image" content="([^"]+)""#, html);

    let mut genres: Vec<String> = Vec::new();
    let mut season = None;
    let mut status = None;
    let mut anime_type = None;
    let mut total_episodes = None;

    let a_sel = sel("a");
    for li in doc.select(&sel(".anime_data ul li")) {
        let text = li.text().collect::<String>();
        if li.value().attr("rel") == Some("tipo") {
            anime_type = Some(clean_text(&text));
            continue;
        }
        if text.contains("Generos") || text.contains("Demografia") {
            for a in li.select(&a_sel) {
                if let Some(href) = a.value().attr("href") {
                    let name = clean_text(&a.text().collect::<String>());
                    if href.contains("/genero/") && !name.is_empty() {
                        genres.push(name);
                    }
                }
            }
        } else if text.contains("Temporada") {
            season = li
                .select(&a_sel)
                .next()
                .map(|a| clean_text(&a.text().collect::<String>()));
        } else if text.contains("Estado") {
            status = Some(clean_text(&text));
        } else if text.contains("Episodios") {
            total_episodes = text
                .split(':')
                .nth(1)
                .and_then(|s| {
                    s.split(|c: char| !c.is_ascii_digit())
                        .find(|d| !d.is_empty())
                })
                .and_then(|d| d.parse::<i32>().ok());
        }
    }

    genres.sort();
    genres.dedup();

    Ok(Anime {
        id: 0,
        slug: slug.to_string(),
        name,
        synopsis,
        season,
        status,
        cover_image,
        total_episodes,
        anime_type,
        url,
        genres,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct EpisodesResponse {
    pub data: Vec<EpisodeItem>,
    #[serde(default)]
    pub last_page: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EpisodeItem {
    pub number: i32,
    #[serde(default)]
    pub title: String,
}

pub fn parse_episodes_response(r: EpisodesResponse) -> Vec<Episode> {
    r.data
        .into_iter()
        .map(|it| Episode {
            id: 0,
            anime_id: 0,
            number: it.number,
            title: if it.title.is_empty() {
                Some(format!("Episodio {}", it.number))
            } else {
                Some(it.title)
            },
            video_url: None,
            duration: None,
        })
        .collect()
}

/// Extrae la URL de video desde la página del reproductor jkplayer.
pub fn extract_video_from_player(html: &str) -> Option<String> {
    if let Some(url) = regex_capture(r#"<source src=['"]([^'"]+)['"]"#, html) {
        return Some(url);
    }
    // formatos alternativos usados por jkplayer/um
    if let Some(url) = regex_capture(r#"file:\s*['"]([^'"]+)['"]"#, html) {
        return Some(url);
    }
    if let Some(url) = regex_capture(r#"src:\s*['"]([^'"]+)['"]"#, html) {
        if url.starts_with("http") {
            return Some(url);
        }
    }
    None
}

/// Encuentra el iframe del reproductor dentro de la página de un episodio.
pub fn find_player_url(html: &str) -> Option<String> {
    regex_capture(r#"(jkplayer/umv\?e=[^"']+)"#, html)
        .or_else(|| regex_capture(r#"(jkplayer/um\?e=[^"']+)"#, html))
}

#[derive(Debug, Deserialize)]
struct CatalogResponse {
    data: Vec<CatalogItem>,
    current_page: u32,
    last_page: u32,
    per_page: u32,
    total: u32,
}

#[derive(Debug, Deserialize)]
struct CatalogItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    synopsis: String,
    #[serde(default)]
    image: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    estado: String,
    #[serde(default)]
    tipo: String,
    #[serde(default)]
    url: String,
}

/// Parsea el catálogo `/directorio` (JSON embebido en `var animes = {...}`).
pub fn parse_catalog(html: &str) -> AppResult<CatalogPage> {
    let json = regex_capture(r"var animes = (\{.*?\});", html)
        .ok_or_else(|| AppError::Provider("no se encontró el catálogo en /directorio".into()))?;
    let parsed: CatalogResponse =
        serde_json::from_str(&json).map_err(|e| AppError::Provider(format!("JSON de catálogo: {e}")))?;

    let items: Vec<Anime> = parsed
        .data
        .into_iter()
        .map(|d| Anime {
            id: 0,
            slug: d.slug,
            name: d.title,
            synopsis: Some(d.synopsis).filter(|s| !s.is_empty()),
            season: None,
            status: Some(d.estado).filter(|s| !s.is_empty()),
            cover_image: Some(d.image),
            total_episodes: None,
            anime_type: Some(d.tipo).filter(|s| !s.is_empty()),
            url: d.url,
            genres: Vec::new(),
        })
        .collect();

    Ok(CatalogPage {
        items,
        page: parsed.current_page,
        total: parsed.total,
        per_page: parsed.per_page,
        last_page: parsed.last_page,
    })
}

/// Construye la URL del directorio a partir de un filtro y una página.
pub fn catalog_url(filter: &CatalogFilter, page: u32) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(g) = &filter.genero {
        params.push(format!("genero={g}"));
    }
    if let Some(d) = &filter.demografia {
        params.push(format!("demografia={d}"));
    }
    if let Some(t) = &filter.temporada {
        params.push(format!("temporada={t}"));
    }
    if let Some(t) = &filter.tipo {
        params.push(format!("tipo={t}"));
    }
    if let Some(e) = &filter.estado {
        params.push(format!("estado={e}"));
    }
    if let Some(a) = filter.anio {
        params.push(format!("fecha={a}"));
    }
    if let Some(o) = &filter.orden {
        params.push(format!("orden={o}"));
    }
    params.push(format!("p={page}"));
    format!("{BASE_URL}/directorio?{}", params.join("&"))
}

/// Animes recientes de la home (`Animes recientes`).
pub fn parse_recent(html: &str) -> AppResult<Vec<Anime>> {
    let marker = "Animes recientes";
    let start = html.find(marker).ok_or_else(|| {
        AppError::Provider("no se encontró la sección de animes recientes en la home".into())
    })?;
    let section = &html[start..];

    let doc = Html::parse_document(section);
    let block_sel =
        Selector::parse(".custom_thumb_home").map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    let a_sel = Selector::parse("a").unwrap();
    let img_sel = Selector::parse("img").unwrap();
    let badge_sel = Selector::parse(".badge").unwrap();
    let title_sel = Selector::parse(".card-title").unwrap();

    let mut out = Vec::new();
    for block in doc.select(&block_sel) {
        let a = block.select(&a_sel).next();
        let href = a.and_then(|e| e.value().attr("href")).map(|s| s.to_string());
        let img = block.select(&img_sel).next();
        let cover = img.and_then(|e| e.value().attr("src")).map(|s| s.to_string());
        let name = block
            .select(&title_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .unwrap_or_default();
        let status = block
            .select(&badge_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .filter(|s| !s.is_empty());

        let Some(href) = href else { continue };
        let Some(slug) = slug_from_url(&href) else { continue };

        out.push(Anime {
            id: 0,
            slug,
            name,
            synopsis: None,
            season: None,
            status,
            cover_image: cover,
            total_episodes: None,
            anime_type: None,
            url: resolve_url(&href, BASE_URL),
            genres: Vec::new(),
        });
    }
    Ok(out)
}

/// Top animes de la home (`Top animes`).
pub fn parse_recommended(html: &str) -> AppResult<Vec<Anime>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(".toplist").map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    let a_sel = Selector::parse("a").unwrap();
    let img_sel = Selector::parse("img").unwrap();
    let title_sel = Selector::parse(".card-title").unwrap();
    let syn_sel = Selector::parse(".card-synopsis").unwrap();

    let mut out = Vec::new();
    for item in doc.select(&sel) {
        let href = item
            .select(&a_sel)
            .next()
            .and_then(|e| e.value().attr("href"))
            .map(|s| s.to_string());
        let cover = item
            .select(&img_sel)
            .next()
            .and_then(|e| e.value().attr("src"))
            .map(|s| s.to_string());
        let name = item
            .select(&title_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .unwrap_or_default();
        let synopsis = item
            .select(&syn_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .filter(|s| !s.is_empty());

        let Some(href) = href else { continue };
        let Some(slug) = slug_from_url(&href) else { continue };

        out.push(Anime {
            id: 0,
            slug,
            name,
            synopsis,
            season: None,
            status: None,
            cover_image: cover,
            total_episodes: None,
            anime_type: None,
            url: resolve_url(&href, BASE_URL),
            genres: Vec::new(),
        });
    }
    Ok(out)
}

/// Lista de géneros desde el directorio.
pub fn parse_genres(html: &str) -> AppResult<Vec<Tag>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(r#"select[name="genero"] option"#)
        .map_err(|e| AppError::Provider(format!("selector: {e}")))?;

    let mut out = Vec::new();
    for opt in doc.select(&sel) {
        let value = opt.value().attr("value").unwrap_or("");
        if value.is_empty() {
            continue;
        }
        let name = clean_text(&opt.text().collect::<String>());
        out.push(Tag {
            id: 0,
            name: name.clone(),
            description: Some(name),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_extraction() {
        assert_eq!(
            slug_from_url("https://jkanime.net/boruto-naruto-next-generations/").as_deref(),
            Some("boruto-naruto-next-generations")
        );
        assert_eq!(slug_from_url("https://jkanime.net/"), None);
    }

    #[test]
    fn clean_text_collapses_whitespace() {
        assert_eq!(clean_text("  Boruto:\n  Naruto "), "Boruto: Naruto");
    }

    #[test]
    fn player_url_extraction() {
        let html = r#"<iframe src="https://jkanime.net/jkplayer/umv?e=Zm5b&t=abc" width="565"></iframe>"#;
        assert_eq!(
            find_player_url(html).as_deref(),
            Some("jkplayer/umv?e=Zm5b&t=abc")
        );
    }

    #[test]
    fn video_source_extraction() {
        let html = r#"<video><source src='https://cdn.example.com/master.m3u8?st=abc&e=123' type='application/x-mpegURL'></video>"#;
        assert_eq!(
            extract_video_from_player(html).as_deref(),
            Some("https://cdn.example.com/master.m3u8?st=abc&e=123")
        );
    }

    #[test]
    fn catalog_url_build() {
        let f = CatalogFilter {
            genero: Some("accion".into()),
            estado: Some("emision".into()),
            anio: Some(2025),
            ..Default::default()
        };
        assert_eq!(
            catalog_url(&f, 3),
            "https://jkanime.net/directorio?genero=accion&estado=emision&fecha=2025&p=3"
        );
    }
}
