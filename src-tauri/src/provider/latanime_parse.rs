use base64::Engine;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

use crate::domain::{Anime, CatalogFilter, CatalogPage, Episode, Tag};
use crate::error::{AppError, AppResult};

pub const BASE_URL: &str = "https://latanime.org";

pub fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn regex_capture(pattern: &str, hay: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    re.captures(hay).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

pub fn regex_captures(pattern: &str, hay: &str) -> Vec<String> {
    let Ok(re) = Regex::new(pattern) else {
        return Vec::new();
    };
    re.captures_iter(hay)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

pub fn slug_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    let slug = trimmed.rsplit('/').next()?;
    if slug.is_empty() || slug.contains('.') || slug == "anime" {
        return None;
    }
    Some(slug.to_string())
}

fn img_attr(el: &ElementRef, img_sel: &Selector, name: &str) -> Option<String> {
    el.select(img_sel)
        .next()
        .and_then(|n| n.value().attr(name))
        .map(|s| s.to_string())
}

/// Muestra de la carátula: prefiere `data-src` (lozad) y cae a `src`.
fn cover_of(el: &ElementRef, img_sel: &Selector) -> Option<String> {
    img_attr(el, img_sel, "data-src")
        .filter(|s| !s.contains("capblank") && !s.contains("/img/anime.png"))
        .or_else(|| img_attr(el, img_sel, "src"))
}

/// Extrae `slug` y `number` de una URL de episodio `/ver/{anime}-episodio-{n}`.
pub fn parse_ver_url(url: &str) -> Option<(String, i32)> {
    let trimmed = url.trim_end_matches('/');
    let re = Regex::new(r"/ver/(.+?)-episodio-(\d+)").ok()?;
    let caps = re.captures(trimmed)?;
    let number = caps.get(2)?.as_str().parse::<i32>().ok()?;
    let slug = caps.get(1)?.as_str().to_string();
    if slug.is_empty() {
        return None;
    }
    Some((slug, number))
}

/// Tarjeta de anime en búsqueda y catálogo (`div.series`).
fn parse_series_card(el: ElementRef) -> Option<Anime> {
    // El enlace es el PADRE de `div.series`; también se acepta un `a`
    // descendiente (layouts alternativos).
    let a_sel = Selector::parse("a[href*='/anime/']").ok()?;
    let href = el
        .parent()
        .and_then(|p| p.value().as_element())
        .and_then(|e| e.attr("href"))
        .map(|s| s.to_string())
        .or_else(|| el.select(&a_sel).next().and_then(|n| n.value().attr("href")).map(|s| s.to_string()))?;
    let slug = slug_from_url(&href)?;

    let title_sel = Selector::parse(".seriedetails h3").ok()?;
    let img_sel = Selector::parse(".serieimg img").ok()?;
    let badge_sel = Selector::parse(".seriedetails div span").ok()?;

    let name = el
        .select(&title_sel)
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .unwrap_or_else(|| slug.clone());

    let cover = cover_of(&el, &img_sel);

    let mut anime_type = None;
    let mut season = None;
    for badge in el.select(&badge_sel) {
        let text = clean_text(&badge.text().collect::<String>());
        if text.is_empty() {
            continue;
        }
        let cls = badge.value().attr("class").unwrap_or("");
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if cls.contains("opacity-75") {
            anime_type = Some(text);
        } else if digits.len() == 4 && season.is_none() {
            season = Some(text);
        }
    }

    Some(Anime {
        id: 0,
        slug: slug.clone(),
        name,
        synopsis: None,
        season,
        status: None,
        cover_image: cover,
        total_episodes: None,
        anime_type,
        url: format!("{BASE_URL}/anime/{slug}"),
        genres: Vec::new(),
    })
}

/// Resultados de `/buscar?q=...`.
pub fn parse_search(html: &str) -> AppResult<Vec<Anime>> {
    let doc = Html::parse_document(html);
    let card_sel =
        Selector::parse("div.series").map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    Ok(doc.select(&card_sel).filter_map(parse_series_card).collect())
}

/// Detalle de un anime desde `/anime/{slug}`.
pub fn parse_anime_detail(html: &str, slug: &str) -> AppResult<Anime> {
    let doc = Html::parse_document(html);
    let sel = |s: &str| Selector::parse(s).expect("selector estático válido");

    let url = format!("{BASE_URL}/anime/{slug}");

    let name = doc
        .select(&sel("div.col-lg-9 h2"))
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .unwrap_or_else(|| slug.to_string());

    let cover_image = regex_capture(r#"property="og:image" content="([^"]+)""#, html)
        .or_else(|| regex_capture(r#"name="og:image" content="([^"]+)""#, html));

    let synopsis = doc
        .select(&sel("p.my-2.opacity-75"))
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .filter(|s| !s.is_empty());

    let status = doc
        .select(&sel("div.series2 button.btn-estado"))
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .filter(|s| !s.is_empty());

    let total_episodes = regex_capture(r"Episodios:\s*(\d+)", html)
        .and_then(|d| d.parse::<i32>().ok());

    let mut genres: Vec<String> = Vec::new();
    let geno_sel = sel(r#"a[href^="/genero/"]"#);
    for a in doc.select(&geno_sel) {
        let name = clean_text(&a.text().collect::<String>());
        if !name.is_empty() {
            genres.push(name);
        }
    }
    genres.sort();
    genres.dedup();

    Ok(Anime {
        id: 0,
        slug: slug.to_string(),
        name,
        synopsis,
        season: None,
        status,
        cover_image,
        total_episodes,
        anime_type: None,
        url,
        genres,
    })
}

fn episode_title_from_text(text: &str, number: i32) -> String {
    let cleaned = regex_capture(r"(?i)(-?\s*Capitulo\s*\d+)\s*$", text.trim())
        .map(|cap| {
            // `cap` es el grupo capturado; quítalo del final del texto.
            text.trim()
                .trim_end_matches(cap.trim())
                .trim()
                .to_string()
        })
        .unwrap_or_else(|| text.trim().to_string());
    if cleaned.is_empty() {
        format!("Episodio {number}")
    } else {
        cleaned
    }
}

/// Lista de episodios desde `/anime/{slug}` (todos renderizados en el HTML).
pub fn parse_episodes(html: &str) -> AppResult<Vec<Episode>> {
    let doc = Html::parse_document(html);

    let mut out: Vec<Episode> = Vec::new();
    let push = |out: &mut Vec<Episode>, number: i32, title: Option<String>| {
        if !out.iter().any(|e| e.number == number) {
            out.push(Episode {
                id: 0,
                anime_id: 0,
                number,
                title,
                video_url: None,
                duration: None,
            });
        }
    };

    // Formato actual: filas `.cap-layout` dentro de `a[href*='/ver/']`.
    let row_sel = Selector::parse(r#"a[href*='/ver/']"#)
        .map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    for a in doc.select(&row_sel) {
        let Some(href) = a.value().attr("href") else { continue };
        let Some((_, number)) = parse_ver_url(href) else { continue };

        let text = clean_text(&a.text().collect::<String>());
        let title = if text.to_lowercase().contains("capitulo") {
            Some(episode_title_from_text(&text, number))
        } else {
            Some(format!("Episodio {number}"))
        };
        push(&mut out, number, title);
    }

    // Formato legado (listados grandes bajo `.jpage .col-item[data-episode]`).
    let legacy_sel = Selector::parse(".jpage .col-item[data-episode]")
        .map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    for it in doc.select(&legacy_sel) {
        let number = it
            .value()
            .attr("data-episode")
            .and_then(|d| d.parse::<i32>().ok())
            .unwrap_or(0);
        if number == 0 {
            continue;
        }
        let text = clean_text(&it.text().collect::<String>());
        let title = Some(format!("Episodio {number}")).filter(|_| text.is_empty() || text.to_lowercase().contains("episodio"));
        push(&mut out, number, title);
    }

    out.sort_by_key(|e| e.number);
    out.dedup_by_key(|e| e.number);
    Ok(out)
}

/// Mirrors de video: decodifica cada `data-player` (base64 → URL de embed).
pub fn player_embed_urls(html: &str) -> Vec<String> {
    use base64::engine::general_purpose::STANDARD as B64;
    regex_captures(r#"data-player="([A-Za-z0-9+/=]+)""#, html)
        .into_iter()
        .filter_map(|b64| B64.decode(b64).ok().and_then(|b| String::from_utf8(b).ok()))
        .collect()
}

/// Ruta relativa `pass_md5/<hash>/<token>` dentro del embed de DoodStream.
pub fn dsvplay_pass_md5_path(embed_html: &str) -> Option<String> {
    regex_capture(r#"\$\.get\(\s*['"/"]?(/pass_md5/[^'"\s)]+)"#, embed_html)
        .or_else(|| regex_capture(r#"/pass_md5/[A-Za-z0-9\-]+/[A-Za-z0-9]+"#, embed_html))
}

/// ID del embed de Hexload (`/embed-<id>`).
pub fn hexload_embed_id(url: &str) -> Option<String> {
    regex_capture(r#"/embed-([A-Za-z0-9]+)"#, url)
}

/// Clasifica un embed para saber qué espejo es.
pub fn mirror_of(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains("dsvplay") {
        MIRROR_DSVPLAY
    } else if lower.contains("hexload") {
        MIRROR_HEXLOAD
    } else {
        MIRROR_OTHER
    }
}

pub const MIRROR_DSVPLAY: &str = "dsvplay";
pub const MIRROR_HEXLOAD: &str = "hexload";
pub const MIRROR_OTHER: &str = "other";

/// Página del directorio `/animes` (contenido HTML; la paginación se deduce
/// de los enlaces `ul.pagination`).
pub fn parse_catalog(html: &str, page: u32) -> AppResult<CatalogPage> {
    let doc = Html::parse_document(html);
    let card_sel =
        Selector::parse("div.series").map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    let items: Vec<Anime> = doc.select(&card_sel).filter_map(parse_series_card).collect();

    let page_links_sel =
        Selector::parse("ul.pagination a.page-link").map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    let mut last_page = page;
    for link in doc.select(&page_links_sel) {
        let text = clean_text(&link.text().collect::<String>());
        if let Ok(n) = text.parse::<u32>() {
            last_page = last_page.max(n);
        }
    }
    let per_page = 30u32.max(items.len() as u32);

    Ok(CatalogPage {
        items,
        page,
        total: last_page * per_page,
        per_page,
        last_page,
    })
}

/// Construye la URL del directorio con filtros y página.
pub fn catalog_url(filter: &CatalogFilter, page: u32) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(g) = &filter.genero {
        params.push(format!("genero={g}"));
    }
    if let Some(c) = &filter.tipo {
        params.push(format!("categoria={c}"));
    }
    if let Some(a) = filter.anio {
        params.push(format!("fecha={a}"));
    }
    if let Some(o) = &filter.orden {
        let o = if o.starts_with("popular") { "popularidad" } else { "nombre" };
        if !params.iter().any(|p| p.starts_with("orden=")) {
            params.push(format!("orden={o}"));
        }
    }
    if !params.is_empty() {
        params.push(format!("p={page}"));
    } else if page > 1 {
        params.push(format!("p={page}"));
    }
    let query = params.join("&");
    if query.is_empty() {
        format!("{BASE_URL}/animes")
    } else {
        format!("{BASE_URL}/animes?{query}")
    }
}

/// Animes recientes de la home: sección "Añadidos recientemente".
pub fn parse_recent(html: &str) -> AppResult<Vec<Anime>> {
    let marker = "Añadidos recientemente";
    let start = html.find(marker).ok_or_else(|| {
        AppError::Provider("no se encontró la sección de recientes en la home".into())
    })?;
    let section = &html[start..];

    let doc = Html::parse_document(section);
    let item_sel = Selector::parse("a[href*='/ver/']")
        .map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    let img_sel = Selector::parse(".imgrec img").unwrap();
    let title_sel = Selector::parse(".info h2").unwrap();
    let dub_sel = Selector::parse(".info_cap span").unwrap();

    let mut out = Vec::new();
    for a in doc.select(&item_sel) {
        let Some(href) = a.value().attr("href") else { continue };
        let Some((slug, _)) = parse_ver_url(href) else { continue };

        let cover = cover_of(&a, &img_sel);
        let name = a
            .select(&title_sel)
            .next()
            .and_then(|h| {
                let text = clean_text(&h.text().collect::<String>());
                let re = Regex::new(r"(?i)^Episodio\s*\d+\s*-\s*").ok()?;
                Some(re.replace(&text, "").trim().to_string())
            })
            .unwrap_or_else(|| slug.clone());
        let dub = a
            .select(&dub_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .filter(|s| !s.is_empty());

        out.push(Anime {
            id: 0,
            slug: slug.clone(),
            name,
            synopsis: None,
            season: None,
            status: None,
            cover_image: cover,
            total_episodes: None,
            anime_type: dub,
            url: format!("{BASE_URL}/anime/{slug}"),
            genres: Vec::new(),
        });
    }
    Ok(out)
}

/// Recomendados de la home: carrusel destacado del hero.
pub fn parse_recommended(html: &str) -> AppResult<Vec<Anime>> {
    let doc = Html::parse_document(html);
    let item_sel = Selector::parse("#carouselExampleCaptions div.carousel-item a[href*='/anime/']")
        .map_err(|e| AppError::Provider(format!("selector: {e}")))?;
    let img_sel = Selector::parse(".hero-item img.preview-image").unwrap();
    let caption_sel = Selector::parse(".span-slider").unwrap();
    let syn_sel = Selector::parse("p.p-slider").unwrap();

    let mut out = Vec::new();
    for a in doc.select(&item_sel) {
        let Some(href) = a.value().attr("href") else { continue };
        let Some(slug) = slug_from_url(href) else { continue };

        let name = a
            .select(&img_sel)
            .next()
            .and_then(|n| n.value().attr("alt"))
            .map(|s| clean_text(s))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                a.select(&caption_sel)
                    .next()
                    .map(|e| clean_text(&e.text().collect::<String>()))
            })
            .unwrap_or_else(|| slug.clone());

        let synopsis = a
            .select(&syn_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .filter(|s| !s.is_empty());

        out.push(Anime {
            id: 0,
            slug: slug.clone(),
            name,
            synopsis,
            season: None,
            status: None,
            cover_image: cover_of(&a, &img_sel),
            total_episodes: None,
            anime_type: None,
            url: format!("{BASE_URL}/anime/{slug}"),
            genres: Vec::new(),
        });
    }
    Ok(out)
}

/// Lista de géneros desde el `<select name="genero">` del directorio.
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
            slug_from_url("https://latanime.org/anime/oshi-no-ko").as_deref(),
            Some("oshi-no-ko")
        );
        assert_eq!(slug_from_url("https://latanime.org/anime"), None);
    }

    #[test]
    fn ver_url_parses_slug_and_number() {
        assert_eq!(
            parse_ver_url("https://latanime.org/ver/youjo-senki-temporada-2-episodio-3"),
            Some(("youjo-senki-temporada-2".to_string(), 3))
        );
    }

    #[test]
    fn player_data_decodes() {
        // base64("https://dsvplay.com/e/axd4zrx40dwq")
        let html = r#"<li id="play-video"><a class="play-video repro-item cap" data-player="aHR0cHM6Ly9kc3ZwbGF5LmNvbS9lL2F4ZDR6cng0MGR3cQ==">dsvplay</a></li>"#;
        let urls = player_embed_urls(html);
        assert_eq!(urls, vec!["https://dsvplay.com/e/axd4zrx40dwq".to_string()]);
    }

    #[test]
    fn dsvplay_path_extraction() {
        let html = r#"
          jwplayer.key = "...";
          $.get('/pass_md5/270269359-181-114-1786553144-9707db60a47c09da337ea782a48088c1/u635xjo2h0zpm5kqlk32591r', function(res){ videoUrl(res); });
        "#;
        assert_eq!(
            dsvplay_pass_md5_path(html).as_deref(),
            Some("/pass_md5/270269359-181-114-1786553144-9707db60a47c09da337ea782a48088c1/u635xjo2h0zpm5kqlk32591r")
        );
    }

    #[test]
    fn hexload_id_extraction() {
        assert_eq!(
            hexload_embed_id("https://hexload.com/embed-htvcdindode5"),
            Some("htvcdindode5".to_string())
        );
    }

    #[test]
    fn mirror_classification() {
        assert_eq!(mirror_of("https://dsvplay.com/e/xyz"), MIRROR_DSVPLAY);
        assert_eq!(mirror_of("https://hexload.com/embed-xyz"), MIRROR_HEXLOAD);
        assert_eq!(mirror_of("https://mega.nz/embed/#!x!y"), MIRROR_OTHER);
    }

    #[test]
    fn catalog_url_build() {
        let f = CatalogFilter {
            genero: Some("accion".into()),
            anio: Some(2024),
            ..Default::default()
        };
        assert_eq!(catalog_url(&f, 2), "https://latanime.org/animes?genero=accion&fecha=2024&p=2");

        let empty = CatalogFilter::default();
        assert_eq!(catalog_url(&empty, 1), "https://latanime.org/animes");
    }

    #[test]
    fn episode_title_strips_trailing_capitulo() {
        assert_eq!(
            episode_title_from_text("[Oshi No Ko] Latino - Capitulo 11", 11),
            "[Oshi No Ko] Latino"
        );
    }
}