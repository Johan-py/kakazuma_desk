use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::error::{AppError, AppResult};

/// Caché en memoria LRU con TTL por entrada.
pub struct TtlCache<V> {
    inner: Mutex<Inner<V>>,
    capacity: usize,
    ttl: Duration,
}

struct Inner<V> {
    lru: lru::LruCache<String, CacheEntry<V>>,
}

struct CacheEntry<V> {
    value: V,
    created: Instant,
}

impl<V> TtlCache<V> {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity = std::num::NonZeroUsize::new(capacity).unwrap_or(std::num::NonZeroUsize::MIN);
        Self {
            inner: Mutex::new(Inner {
                lru: lru::LruCache::new(capacity),
            }),
            capacity: capacity.get(),
            ttl,
        }
    }

    pub fn get(&self, key: &str) -> Option<V>
    where
        V: Clone,
    {
        let mut inner = self.inner.lock().ok()?;
        let entry = inner.lru.get(key)?;
        if entry.created.elapsed() > self.ttl {
            inner.lru.pop(key);
            return None;
        }
        Some(entry.value.clone())
    }

    pub fn put(&self, key: String, value: V) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.lru.put(key, CacheEntry { value, created: Instant::now() });
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|i| i.lru.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Caché en disco basada en archivos JSON con TTL (mtime).
pub struct DiskCache {
    dir: PathBuf,
    ttl: Duration,
}

impl DiskCache {
    pub fn new(base_dir: &Path, name: &str, ttl: Duration) -> AppResult<Self> {
        let dir = base_dir.join(name);
        std::fs::create_dir_all(&dir).map_err(|e| AppError::Cache(format!("no se pudo crear {dir:?}: {e}")))?;
        Ok(Self { dir, ttl })
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{hex}.json"))
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let path = self.path_for(key);
        if !path.exists() {
            return None;
        }
        let mtime = SystemTime::now()
            .duration_since(path.metadata().ok()?.modified().ok()?)
            .unwrap_or_default();
        if mtime > self.ttl {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str(&raw) {
            Ok(v) => Some(v),
            Err(_) => {
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    pub fn put<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let path = self.path_for(key);
        let raw = serde_json::to_vec(value).map_err(|e| AppError::Cache(format!("serializar: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw).map_err(|e| AppError::Cache(format!("escribir {path:?}: {e}")))?;
        std::fs::rename(&tmp, &path).map_err(|e| AppError::Cache(format!("renombrar {path:?}: {e}")))?;
        debug!(path = %path.display(), "caché disco escrita");
        Ok(())
    }

    pub fn clear(&self) -> AppResult<()> {
        let count = std::fs::read_dir(&self.dir)
            .map_err(|e| AppError::Cache(format!("leer {dir:?}: {e}", dir = self.dir)))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name().to_string_lossy().ends_with(".json")
                    && std::fs::remove_file(e.path()).is_ok()
            })
            .count();
        debug!(count, "caché disco limpiada");
        Ok(())
    }
}

/// Mapa de TTLs por dominio de caché (utilizado por los servicios).
pub struct CacheRegistry {
    ttl: HashMap<String, Duration>,
}

impl CacheRegistry {
    pub fn new() -> Self {
        Self { ttl: HashMap::new() }
    }

    pub fn register(&mut self, name: &str, ttl: Duration) {
        self.ttl.insert(name.to_string(), ttl);
    }

    pub fn ttl(&self, name: &str) -> Duration {
        self.ttl
            .get(name)
            .copied()
            .unwrap_or(Duration::from_secs(900))
    }
}

impl Default for CacheRegistry {
    fn default() -> Self {
        Self::new()
    }
}
