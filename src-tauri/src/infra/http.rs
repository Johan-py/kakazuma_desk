use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

use crate::error::{AppError, AppResult};

const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub user_agent: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub retry_count: u32,
    pub retry_base_delay_ms: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_UA.to_string(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(45),
            retry_count: 3,
            retry_base_delay_ms: 500,
        }
    }
}

/// Cliente HTTP compartido con keep-alive, HTTP/2 y cookies.
#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    config: HttpConfig,
}

impl HttpClient {
    pub fn new(config: HttpConfig) -> AppResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(&config.user_agent).unwrap_or(HeaderValue::from_static(DEFAULT_UA)),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/json;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );
        headers.insert(reqwest::header::ACCEPT_LANGUAGE, HeaderValue::from_static("es-ES,es;q=0.9,en;q=0.8"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .tcp_keepalive(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .http2_adaptive_window(true)
            .cookie_store(true)
            .build()
            .map_err(|e| AppError::Config(format!("no se pudo construir el cliente HTTP: {e}")))?;

        Ok(Self { client, config })
    }

    fn should_retry(status: Option<StatusCode>) -> bool {
        match status {
            None => true, // error de red/timeout
            Some(s) => s.is_server_error() || s == StatusCode::TOO_MANY_REQUESTS || s == StatusCode::REQUEST_TIMEOUT,
        }
    }

    /// GET con retry exponencial; devuelve el body como String.
    pub async fn get_text(&self, url: &str) -> AppResult<String> {
        let body = self.get_with_retry(url).await?;
        String::from_utf8(body).map_err(|e| AppError::Http(format!("respuesta no UTF-8: {e}")))
    }

    /// GET con retry exponencial; devuelve el body como bytes.
    pub async fn get_bytes(&self, url: &str) -> AppResult<Vec<u8>> {
        self.get_with_retry(url).await
    }

    /// GET y deserializa JSON.
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> AppResult<T> {
        let bytes = self.get_with_retry(url).await?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::Http(format!("JSON inválido desde {url}: {e}")))
    }

    /// POST de formulario (para endpoints AJAX con token CSRF).
    pub async fn post_form(
        &self,
        url: &str,
        fields: &[(&str, String)],
        headers: &[(&str, &str)],
    ) -> AppResult<String> {
        let mut attempt = 0u32;
        loop {
            let mut req = self.client.post(url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            let req = req.form(&fields.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
            let resp = req.send().await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) if attempt < self.config.retry_count => {
                    let delay = self.backoff(attempt);
                    warn!(url, attempt, delay_ms = delay.as_millis(), error = %e, "retry POST (red)");
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(AppError::Http(format!("POST falló: {e}"))),
            };

            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| AppError::Http(format!("lectura de respuesta falló: {e}")))?;

            if Self::should_retry(Some(status)) && attempt < self.config.retry_count {
                let delay = self.backoff(attempt);
                warn!(url, status = %status, attempt, "retry POST (HTTP)");
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            if status != StatusCode::OK {
                return Err(AppError::Http(format!("POST {url} devolvió {status}")));
            }
            return Ok(text);
        }
    }

    async fn get_with_retry(&self, url: &str) -> AppResult<Vec<u8>> {
        let mut attempt = 0u32;
        loop {
            let resp = self.client.get(url).send().await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) if attempt < self.config.retry_count => {
                    let delay = self.backoff(attempt);
                    warn!(url, attempt, delay_ms = delay.as_millis(), error = %e, "retry GET (red)");
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(AppError::Http(format!("GET {url} falló: {e}"))),
            };

            let status = resp.status();
            if Self::should_retry(Some(status)) && attempt < self.config.retry_count {
                let delay = self.backoff(attempt);
                warn!(url, status = %status, attempt, "retry GET (HTTP)");
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            if status == StatusCode::NOT_FOUND {
                return Err(AppError::NotFound(url.to_string()));
            }
            if status != StatusCode::OK {
                return Err(AppError::Http(format!("GET {url} devolvió {status}")));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::Http(format!("lectura de {url} falló: {e}")))?;
            debug!(url, bytes = bytes.len(), "GET ok");
            return Ok(bytes.to_vec());
        }
    }

    /// Backoff exponencial con jitter determinista (0..=base * 2^attempt).
    fn backoff(&self, attempt: u32) -> Duration {
        let base = self.config.retry_base_delay_ms;
        let exp = base << attempt.min(5);
        let jitter = (attempt as u64) * 137 % (exp + 1);
        Duration::from_millis(exp + jitter)
    }
}

/// URL absoluta respecto a una base (jkanime.net por defecto).
pub fn resolve_url(raw: &str, base: &str) -> String {
    match Url::parse(raw) {
        Ok(_) => raw.to_string(),
        Err(_) => {
            if let Ok(base_url) = Url::parse(base) {
                if let Ok(joined) = base_url.join(raw) {
                    return joined.to_string();
                }
            }
            format!("{base}{raw}")
        }
    }
}

/// Mantiene las cabeceras CSRF usadas en peticiones AJAX.
#[derive(Debug, Clone)]
pub struct CsrfSession {
    pub token: String,
    pub cookie: String,
}

impl CsrfSession {
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let token = headers
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())?;
        let cookie = headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())?;
        Some(Self { token, cookie })
    }
}

pub fn csrf_headers<'a>(session: &'a CsrfSession, referer: &'a str) -> Vec<(&'static str, &'a str)> {
    vec![
        ("X-Requested-With", "XMLHttpRequest"),
        ("X-CSRF-TOKEN", &session.token),
        ("cookie", &session.cookie),
        ("referer", referer),
    ]
}
