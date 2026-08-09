use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use crate::error::{AppError, AppResult};

/// Servidor HTTP embebido (localhost) que sirve los segmentos cacheados a mpv.
///
/// Expone las rutas `/buffer/<hash>/<number>/<archivo>` mapeadas dentro del
/// directorio raíz del cache de segmentos. Se usa para la reproducción
/// local-first: la playlist híbrida referencia segmentos de forma relativa y
/// mpv los descarga desde aquí a velocidad de disco en lugar de la red.
pub struct SegmentServer {
    port: u16,
    shutdown: Arc<AtomicBool>,
}

impl SegmentServer {
    /// Arranca el servidor en `127.0.0.1` con puerto efímero. Devuelve la
    /// instancia y el puerto real asignado.
    pub fn start(cache_dir: PathBuf) -> AppResult<(Arc<Self>, u16)> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| AppError::Cache(format!("no se pudo abrir el servidor local: {e}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| AppError::Cache(format!("servidor local no bloqueante: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| AppError::Cache(format!("sin puerto local: {e}")))?
            .port();

        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        std::thread::Builder::new()
            .name("kakasuma-segserver".into())
            .spawn(move || accept_loop(listener, cache_dir, thread_shutdown))
            .map_err(|e| AppError::Cache(format!("no se pudo crear el hilo del servidor local: {e}")))?;

        debug!(port, "servidor de segmentos local escuchando");
        Ok((Arc::new(Self { port, shutdown }), port))
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

fn accept_loop(listener: TcpListener, cache_dir: PathBuf, shutdown: Arc<AtomicBool>) {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let cache = cache_dir.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, &cache);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                warn!(error = %e, "aceptando conexión en el servidor local");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_conn(mut stream: TcpStream, cache_dir: &Path) -> AppResult<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));

    let mut request = Vec::with_capacity(4096);
    let mut buf = [0u8; 4096];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        request.extend_from_slice(&buf[..n]);
        if request.windows(4).any(|w| w == b"\r\n\r\n") || request.windows(2).any(|w| w == b"\n\n")
        {
            break;
        }
        if request.len() > 64 * 1024 {
            break;
        }
    }

    let text = String::from_utf8_lossy(&request);
    let mut lines = text.lines();
    let status_line = lines.next().unwrap_or_default();
    let mut parts = status_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    let mut range: Option<String> = None;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("range:") {
            range = Some(rest.trim().to_string());
        }
    }

    let Some(rel) = path.strip_prefix("/buffer/") else {
        respond(&mut stream, 404, "Not Found", &[], "text/plain", None);
        return Ok(());
    };
    if rel.contains("..") || rel.contains('\0') {
        respond(&mut stream, 403, "Forbidden", &[], "text/plain", None);
        return Ok(());
    }

    let file = cache_dir.join(rel);
    let canonical = match std::fs::canonicalize(&file) {
        Ok(c) => c,
        Err(_) => {
            respond(&mut stream, 404, "Not Found", &[], "text/plain", None);
            return Ok(());
        }
    };
    let root = cache_dir.canonicalize().unwrap_or_else(|_| cache_dir.to_path_buf());
    if !canonical.starts_with(&root) {
        respond(&mut stream, 403, "Forbidden", &[], "text/plain", None);
        return Ok(());
    }
    if !canonical.is_file() {
        respond(&mut stream, 404, "Not Found", &[], "text/plain", None);
        return Ok(());
    }

    serve_file(&mut stream, &canonical, method, range.as_deref())
}

fn serve_file(stream: &mut TcpStream, path: &Path, method: &str, range: Option<&str>) -> AppResult<()> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            warn!(file = %path.display(), error = %e, "leyendo archivo del buffer local");
            respond(stream, 500, "Internal Server Error", &[], "text/plain", None);
            return Ok(());
        }
    };
    let ctype = content_type(path);
    let total = data.len();

    match range {
        Some(r) if r.starts_with("bytes=") => {
            let spec = &r["bytes=".len()..];
            let (start, end) = parse_range(spec, total);
            if start >= total {
                respond(stream, 416, "Range Not Satisfiable", &[], "text/plain", None);
                return Ok(());
            }
            let end = end.min(total - 1);
            let body = &data[start..=end];
            let extra = format!("Content-Range: bytes {start}-{end}/{total}");
            respond(stream, 206, "Partial Content", body, ctype, Some(&extra));
        }
        _ => {
            if method == "HEAD" {
                respond(stream, 200, "OK", &[], ctype, None);
            } else {
                respond(stream, 200, "OK", &data, ctype, None);
            }
        }
    }
    Ok(())
}

fn parse_range(spec: &str, total: usize) -> (usize, usize) {
    let spec = spec.trim();
    let mut it = spec.splitn(2, '-');
    let a = it.next().unwrap_or("").trim();
    let b = it.next().unwrap_or("").trim();
    match (a, b) {
        ("", "") => (0, total.saturating_sub(1)),
        (a, "") => (a.parse().unwrap_or(0), total.saturating_sub(1)),
        ("", b) => {
            let suffix: usize = b.parse().unwrap_or(0);
            let start = total.saturating_sub(suffix);
            (start, total.saturating_sub(1))
        }
        (a, b) => {
            let start = a.parse().unwrap_or(0);
            let end = b.parse().unwrap_or(total.saturating_sub(1));
            (start, end)
        }
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("ts") => "video/mp2t",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

fn respond(
    stream: &mut TcpStream,
    code: u16,
    reason: &str,
    body: &[u8],
    ctype: &str,
    extra_header: Option<&str>,
) {
    let mut head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    if let Some(h) = extra_header {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut out = Vec::with_capacity(head.len() + body.len());
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(body);
    let _ = stream.write_all(&out);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_get(port: u16, path: &str, extra: &str) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap();
        let text = String::from_utf8_lossy(&buf);
        let head_end = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(text.len());
        let head = text[..head_end].to_string();
        let body = buf[head_end.min(buf.len())..].to_vec();
        (head, body)
    }

    #[test]
    fn serves_file_with_content_type() {
        let dir = std::env::temp_dir().join(format!("segserver-test-{}", std::process::id()));
        let ep = dir.join("hash").join("1");
        std::fs::create_dir_all(&ep).unwrap();
        std::fs::write(ep.join("seg_0000.ts"), b"TSDATA123").unwrap();

        let (server, port) = SegmentServer::start(dir.clone()).unwrap();
        let (head, body) = raw_get(port, "/buffer/hash/1/seg_0000.ts", "");
        assert!(head.starts_with("HTTP/1.1 200"));
        assert!(head.contains("video/mp2t"));
        assert_eq!(body, b"TSDATA123");
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = std::env::temp_dir().join(format!("segserver-trav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("secret.txt"), b"top").unwrap();

        let (server, port) = SegmentServer::start(dir.clone()).unwrap();
        let (head, _) = raw_get(port, "/buffer/../../secret.txt", "");
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_range_request() {
        let dir = std::env::temp_dir().join(format!("segserver-range-{}", std::process::id()));
        let ep = dir.join("hash").join("2");
        std::fs::create_dir_all(&ep).unwrap();
        std::fs::write(ep.join("seg_0001.ts"), b"0123456789").unwrap();

        let (server, port) = SegmentServer::start(dir.clone()).unwrap();
        let (head, body) = raw_get(port, "/buffer/hash/2/seg_0001.ts", "Range: bytes=2-5\r\n");
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert!(head.contains("Content-Range: bytes 2-5/10"), "{head}");
        assert_eq!(body, b"2345");
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_found_returns_404() {
        let dir = std::env::temp_dir().join(format!("segserver-404-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let (server, port) = SegmentServer::start(dir.clone()).unwrap();
        let (head, _) = raw_get(port, "/buffer/nope/nope.ts", "");
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
