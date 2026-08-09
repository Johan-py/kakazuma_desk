use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};
use url::Url;

use crate::error::{AppError, AppResult};
use crate::infra::http::HttpClient;

/// Número máximo de reintentos por segmento descargado.
pub const MAX_SEGMENT_RETRIES: u32 = 3;
/// Tamaño de chunk usado para el throttling por ancho de banda.
pub const CHUNK_SIZE: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlsSegment {
    /// URI absoluta del segmento.
    pub uri: String,
    pub duration: f64,
    pub byte_range: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlsKey {
    pub method: String,
    /// URI absoluta de la clave (resuelta contra la base del nivel).
    pub uri: Option<String>,
    pub iv: Option<String>,
}

/// Playlist de nivel (media playlist) ya normalizada.
#[derive(Debug, Clone, Default)]
pub struct LevelPlaylist {
    pub segments: Vec<HlsSegment>,
    /// Clave que aplica a cada segmento (índice paralelo).
    pub keys: Vec<Option<HlsKey>>,
    pub target_duration: f64,
    pub end_list: bool,
    /// Base URL para resolver URIs relativas del nivel.
    pub base_url: String,
    pub supports_buffer: bool,
    pub unsupported_reason: Option<&'static str>,
}

/// Resultado de resolver un maestro m3u8.
pub enum HlsFetch {
    Unsupported(&'static str),
    Ready { level: LevelPlaylist },
}

/// Cliente HLS: parseo de playlists, descarga de segmentos y construcción de
/// playlists híbridas (segmentos locales + tail remoto).
pub struct HlsClient {
    http: HttpClient,
}

impl HlsClient {
    pub fn new(http: HttpClient) -> Self {
        Self { http }
    }

    /// Descarga el maestro y produce la playlist de nivel a cachear.
    ///
    /// - Si el maestro contiene variantes (`#EXT-X-STREAM-INF`), elige la de
    ///   mayor resolución (desempate por bitrate).
    /// - Rechaza playlists vivas (sin `#EXT-X-ENDLIST`), `#EXT-X-BYTERANGE`
    ///   y fMP4 (`#EXT-X-MAP`).
    pub async fn resolve_level(&self, master_url: &str) -> AppResult<HlsFetch> {
        let body = self.http.get_text(master_url).await?;
        let variants = parse_master(&body, master_url);

        let (level_url, base_url) = match choose_variant(&variants) {
            Some(v) => (v.uri.clone(), v.uri.as_str()),
            None => (master_url.to_string(), master_url),
        };

        let level_body = self.http.get_text(&level_url).await?;
        let level = parse_level(&level_body, base_url);

        if !level.end_list {
            return Ok(HlsFetch::Unsupported("live"));
        }
        if !level.supports_buffer {
            let reason = level.unsupported_reason.unwrap_or("unsupported");
            return Ok(HlsFetch::Unsupported(reason));
        }
        debug!(segments = level.segments.len(), level_url, "playlist HLS lista para buffer");
        Ok(HlsFetch::Ready { level })
    }

    /// Nº de segmentos a precargar para un porcentaje dado. Nunca devuelve el
    /// total (el 100 % nunca se cachea).
    pub fn segment_range(len: usize, percent: u32) -> usize {
        if len <= 1 {
            return 0;
        }
        let n = (len as u64 * percent as u64 / 100) as usize;
        n.clamp(1, len - 1)
    }

    /// Construye una playlist híbrida: segmentos locales (rutas absolutas)
    /// seguidos del tail remoto. Incluye `EXT-X-KEY` si el tail está cifrado.
    pub fn build_hybrid_playlist(
        local: &[(PathBuf, f64)],
        tail: &[HlsSegment],
        tail_key: Option<&HlsKey>,
        target_duration: f64,
    ) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("#EXTM3U\n");
        out.push_str("#EXT-X-VERSION:3\n");
        let td = (target_duration.max(1.0)).ceil() as u64;
        out.push_str(&format!("#EXT-X-TARGETDURATION:{td}\n"));
        out.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");

        if let Some(k) = tail_key {
            if k.method.eq_ignore_ascii_case("AES-128") {
                if let Some(uri) = &k.uri {
                    let mut line = format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{uri}\"");
                    if let Some(iv) = &k.iv {
                        line.push_str(&format!(",IV={iv}"));
                    }
                    line.push('\n');
                    out.push_str(&line);
                }
            }
        }

        for (path, dur) in local {
            out.push_str(&format!("#EXTINF:{dur:.3},\n{}\n", path.display()));
        }
        for seg in tail {
            out.push_str(&format!("#EXTINF:{:.3},\n{}\n", seg.duration, seg.uri));
        }
        out.push_str("#EXT-X-ENDLIST\n");
        out
    }

    /// Descarga un segmento aplicando throttling por ancho de banda y
    /// comprobando pausa/cancelación por chunk. Devuelve los bytes escritos.
    pub async fn download_segment(
        &self,
        uri: &str,
        out_path: &Path,
        bucket: &tokio::sync::Mutex<TokenBucket>,
        pause: &std::sync::atomic::AtomicU64,
        shutdown: &std::sync::atomic::AtomicBool,
    ) -> AppResult<u64> {
        let mut attempt = 0u32;
        let tmp = out_path.with_extension("ts.tmp");
        loop {
            if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(AppError::Http("descarga cancelada".into()));
            }
            let resp = match self.http.get_response(uri).await {
                Ok(r) => r,
                Err(e) if attempt < MAX_SEGMENT_RETRIES => {
                    attempt += 1;
                    let delay = std::time::Duration::from_millis(500 * (1u64 << (attempt - 1)));
                    warn!(uri, attempt, delay_ms = delay.as_millis(), error = %e, "retry segmento");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e),
            };

            let mut file = match tokio::fs::File::create(&tmp).await {
                Ok(f) => f,
                Err(e) => {
                    return Err(AppError::Cache(format!("crear archivo temporal {tmp:?}: {e}")));
                }
            };

            let mut stream = resp.bytes_stream();
            let mut total: u64 = 0;
            let mut failed: Option<AppError> = None;

            while let Some(item) = stream.next().await {
                if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(AppError::Http("descarga cancelada".into()));
                }
                wait_if_paused(pause, shutdown).await;

                match item {
                    Ok(bytes) => {
                        let len = bytes.len() as u64;
                        if len > 0 {
                            let mut b = bucket.lock().await;
                            b.acquire(len).await;
                            drop(b);
                            if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut file, &bytes).await {
                                failed = Some(AppError::Cache(format!("escribir segmento: {e}")));
                                break;
                            }
                            total += len;
                        }
                    }
                    Err(e) => {
                        failed = Some(AppError::Http(format!("leer segmento {uri}: {e}")));
                        break;
                    }
                }
            }

            let _ = file.flush().await;
            drop(file);

            if let Some(e) = failed {
                let _ = tokio::fs::remove_file(&tmp).await;
                if attempt < MAX_SEGMENT_RETRIES {
                    attempt += 1;
                    let delay = std::time::Duration::from_millis(500 * (1u64 << (attempt - 1)));
                    warn!(uri, attempt, error = %e, "retry segmento (lectura)");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(e);
            }

            if let Err(e) = tokio::fs::rename(&tmp, out_path).await {
                return Err(AppError::Cache(format!("finalizar segmento {out_path:?}: {e}")));
            }
            return Ok(total);
        }
    }
}

/// Espera mientras haya alguna condición de pausa activa o se solicite apagado.
pub async fn wait_if_paused(pause: &std::sync::atomic::AtomicU64, shutdown: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    while pause.load(Ordering::Relaxed) != 0 && !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Bucket de tokens para limitar el ancho de banda de descarga.
pub struct TokenBucket {
    tokens: f64,
    capacity: f64,
    rate: f64,
    last: std::time::Instant,
}

impl TokenBucket {
    pub fn new(mbps: u64) -> Self {
        let rate = mbps.saturating_mul(1_000_000) as f64 / 8.0;
        Self {
            tokens: rate,
            capacity: rate,
            rate,
            last: std::time::Instant::now(),
        }
    }

    pub fn set_rate(&mut self, mbps: u64) {
        self.rate = mbps.saturating_mul(1_000_000) as f64 / 8.0;
        self.capacity = self.rate;
        self.tokens = self.tokens.min(self.capacity);
    }

    /// Consume `amount` bytes esperando el tiempo necesario.
    pub async fn acquire(&mut self, amount: u64) {
        loop {
            let now = std::time::Instant::now();
            let dt = now.duration_since(self.last).as_secs_f64();
            self.tokens = (self.tokens + dt * self.rate).min(self.capacity);
            self.last = now;
            if self.tokens >= amount as f64 {
                self.tokens -= amount as f64;
                return;
            }
            let deficit = amount as f64 - self.tokens;
            let wait = (deficit / self.rate).min(0.25);
            tokio::time::sleep(std::time::Duration::from_secs_f64(wait.max(0.001))).await;
        }
    }
}

/// URI de la primera clave AES-128 de una playlist (si existe).
pub fn first_key_uri(level: &LevelPlaylist) -> Option<String> {
    level
        .keys
        .iter()
        .flatten()
        .find(|k| k.method.eq_ignore_ascii_case("AES-128"))
        .and_then(|k| k.uri.clone())
}

/// Compara dos URIs de clave ignorando la query string.
pub fn key_uris_match(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(ua), Ok(ub)) => {
            ua.scheme() == ub.scheme()
                && ua.host_str() == ub.host_str()
                && ua.port_or_known_default() == ub.port_or_known_default()
                && ua.path() == ub.path()
        }
        _ => a == b,
    }
}

// ---------- parsing de playlists ----------

#[derive(Debug, Clone)]
struct Variant {
    uri: String,
    bandwidth: u64,
    width: u32,
}

fn parse_attributes(attrs: &str) -> Vec<(String, String)> {
    attrs
        .split(',')
        .filter_map(|kv| {
            let mut it = kv.splitn(2, '=');
            let k = it.next()?.trim();
            let v = it.next()?.trim().trim_matches('"');
            if k.is_empty() {
                None
            } else {
                Some((k.to_string(), v.to_string()))
            }
        })
        .collect()
}

fn parse_master(body: &str, base_url: &str) -> Vec<Variant> {
    let mut variants = Vec::new();
    let mut pending: Option<(u64, u32)> = None;
    for raw in body.lines() {
        let line = raw.trim();
        if line.starts_with("#EXT-X-STREAM-INF:") {
            let attrs = parse_attributes(&line["#EXT-X-STREAM-INF:".len()..]);
            let bw = attrs
                .iter()
                .find(|(k, _)| k == "BANDWIDTH")
                .and_then(|(_, v)| v.parse::<u64>().ok())
                .unwrap_or(0);
            let width = attrs
                .iter()
                .find(|(k, _)| k == "RESOLUTION")
                .and_then(|(_, v)| v.split('x').next())
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            pending = Some((bw, width));
        } else if !line.starts_with('#') && !line.is_empty() {
            if let Some((bw, width)) = pending.take() {
                let uri = resolve_hls_url(line, base_url);
                variants.push(Variant { uri, bandwidth: bw, width });
            }
        }
    }
    variants
}

fn choose_variant(variants: &[Variant]) -> Option<&Variant> {
    variants
        .iter()
        .max_by(|a, b| (a.width, a.bandwidth).cmp(&(b.width, b.bandwidth)))
}

fn parse_level(body: &str, base_url: &str) -> LevelPlaylist {
    let mut pl = LevelPlaylist {
        base_url: base_url.to_string(),
        ..Default::default()
    };
    let mut cur_key: Option<HlsKey> = None;
    let mut cur_duration: f64 = 0.0;

    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if let Some(rest) = line.strip_prefix("#EXTINF:") {
                cur_duration = rest
                    .split(',')
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
                pl.target_duration = rest.trim().parse::<f64>().unwrap_or(0.0);
            } else if line == "#EXT-X-ENDLIST" {
                pl.end_list = true;
            } else if line.starts_with("#EXT-X-KEY:") {
                cur_key = parse_key(&line["#EXT-X-KEY:".len()..], base_url);
            } else if line.starts_with("#EXT-X-BYTERANGE") {
                pl.supports_buffer = false;
                pl.unsupported_reason = Some("byterange");
            } else if line.starts_with("#EXT-X-MAP") {
                pl.supports_buffer = false;
                pl.unsupported_reason = Some("fmp4");
            }
            continue;
        }

        // línea de URI de segmento
        let seg = HlsSegment {
            uri: resolve_hls_url(line, base_url),
            duration: cur_duration,
            byte_range: None,
        };
        pl.keys.push(cur_key.clone());
        pl.segments.push(seg);
        cur_duration = 0.0;
    }

    // Solo es bufferizable si terminó (`#EXT-X-ENDLIST`) y no presenta
    // características no soportadas (byterange/fMP4) detectadas arriba.
    pl.supports_buffer = pl.end_list && pl.unsupported_reason.is_none();
    if !pl.supports_buffer && pl.unsupported_reason.is_none() {
        pl.unsupported_reason = Some("live");
    }
    pl
}

fn parse_key(attrs: &str, base_url: &str) -> Option<HlsKey> {
    let map = parse_attributes(attrs);
    let method = map
        .iter()
        .find(|(k, _)| k == "METHOD")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "NONE".to_string());
    let uri = map
        .iter()
        .find(|(k, _)| k == "URI")
        .and_then(|(_, v)| if v.is_empty() { None } else { Some(resolve_hls_url(v, base_url)) });
    let iv = map
        .iter()
        .find(|(k, _)| k == "IV")
        .map(|(_, v)| v.clone());
    Some(HlsKey { method, uri, iv })
}

fn resolve_hls_url(raw: &str, base_url: &str) -> String {
    match Url::parse(raw) {
        Ok(_) => raw.to_string(),
        Err(_) => match Url::parse(base_url) {
            Ok(base) => base
                .join(raw)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| format!("{base_url}{raw}")),
            Err(_) => format!("{base_url}{raw}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_level() -> &'static str {
        "#EXTM3U\n\
         #EXT-X-VERSION:3\n\
         #EXT-X-TARGETDURATION:10\n\
         #EXT-X-MEDIA-SEQUENCE:0\n\
         #EXTINF:8.5,\n\
         seg0.ts\n\
         #EXTINF:9.2,\n\
         seg1.ts\n\
         #EXTINF:7.8,\n\
         seg2.ts\n\
         #EXT-X-ENDLIST\n"
    }

    #[test]
    fn parses_level() {
        let pl = parse_level(sample_level(), "https://cdn.example.com/ep/");
        assert!(pl.end_list);
        assert_eq!(pl.segments.len(), 3);
        assert_eq!(pl.target_duration, 10.0);
        assert!(pl.supports_buffer);
        assert_eq!(pl.segments[0].uri, "https://cdn.example.com/ep/seg0.ts");
        assert_eq!(pl.segments[0].duration, 8.5);
    }

    #[test]
    fn rejects_live_playlist() {
        let body = sample_level().replace("#EXT-X-ENDLIST\n", "");
        let pl = parse_level(&body, "https://cdn.example.com/ep/");
        assert!(!pl.end_list);
    }

    #[test]
    fn rejects_byterange() {
        let body = "#EXTM3U\n#EXTINF:8.5,\n#EXT-X-BYTERANGE:100@0\nseg0.mp4\n";
        let pl = parse_level(body, "https://cdn.example.com/");
        assert!(!pl.supports_buffer);
    }

    #[test]
    fn parses_key() {
        let body = "#EXTM3U\n\
         #EXT-X-KEY:METHOD=AES-128,URI=\"keys/key.bin\",IV=0x112233\n\
         #EXTINF:8.5,\n\
         seg0.ts\n";
        let pl = parse_level(body, "https://cdn.example.com/ep/");
        let key = pl.keys[0].as_ref().unwrap();
        assert_eq!(key.method, "AES-128");
        assert_eq!(key.uri.as_deref(), Some("https://cdn.example.com/ep/keys/key.bin"));
        assert_eq!(key.iv.as_deref(), Some("0x112233"));
    }

    #[test]
    fn parses_master_variants() {
        let body = "#EXTM3U\n\
         #EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\n\
         l720.m3u8\n\
         #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n\
         l1080.m3u8\n";
        let variants = parse_master(body, "https://cdn.example.com/ep/master.m3u8");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[1].width, 1920);
        let chosen = choose_variant(&variants).unwrap();
        assert_eq!(chosen.uri, "https://cdn.example.com/ep/l1080.m3u8");
    }

    #[test]
    fn segment_range_never_full() {
        assert_eq!(HlsClient::segment_range(10, 20), 2);
        assert_eq!(HlsClient::segment_range(10, 90), 9);
        assert_eq!(HlsClient::segment_range(5, 50), 2);
        assert_eq!(HlsClient::segment_range(1, 50), 0);
        assert_eq!(HlsClient::segment_range(0, 50), 0);
    }

    #[test]
    fn hybrid_playlist_orders_local_then_remote() {
        let local = vec![(PathBuf::from("/tmp/seg_0000.ts"), 8.5)];
        let tail = vec![HlsSegment {
            uri: "https://cdn.example.com/seg1.ts".into(),
            duration: 9.2,
            byte_range: None,
        }];
        let pl = HlsClient::build_hybrid_playlist(&local, &tail, None, 10.0);
        let local_pos = pl.find("/tmp/seg_0000.ts").unwrap();
        let remote_pos = pl.find("https://cdn.example.com/seg1.ts").unwrap();
        assert!(local_pos < remote_pos);
        assert!(pl.ends_with("#EXT-X-ENDLIST\n"));
    }

    #[test]
    fn key_match_ignores_query() {
        assert!(key_uris_match(
            "https://cdn.example.com/k.bin?st=old",
            "https://cdn.example.com/k.bin?st=new"
        ));
        assert!(!key_uris_match(
            "https://cdn.example.com/k.bin?st=old",
            "https://cdn.example.com/other.bin?st=new"
        ));
    }
}
