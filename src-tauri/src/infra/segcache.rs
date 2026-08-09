use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use tracing::debug;

use crate::error::{AppError, AppResult};

/// Tras superar el límite se purga hasta el 80 % del mismo.
pub const PURGE_TARGET_RATIO: f64 = 0.8;

/// Caché en disco de segmentos HLS precargados.
///
/// Estructura de directorios:
/// `{dir}/{sha256(slug)}/{number}/seg_{i:04}.ts` + `manifest.json`.
/// El tamaño total se mantiene en un contador atómico y la evicción es LRU
/// por mtime cuando se supera el límite configurado.
pub struct SegmentCache {
    dir: PathBuf,
    bytes: AtomicU64,
    limit_bytes: AtomicU64,
}

impl SegmentCache {
    pub fn new(base: &Path, name: &str) -> AppResult<Self> {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Cache(format!("no se pudo crear {dir:?}: {e}")))?;
        let bytes = Self::scan_bytes(&dir);
        debug!(dir = %dir.display(), bytes, "caché de segmentos lista");
        Ok(Self {
            dir,
            bytes: AtomicU64::new(bytes),
            limit_bytes: AtomicU64::new(0),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Directorio estable por serie (hash del slug).
    pub fn slug_dir(&self, slug: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(slug.as_bytes());
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(hex)
    }

    /// Directorio de un episodio concreto.
    pub fn episode_dir(&self, slug: &str, number: i32) -> PathBuf {
        self.slug_dir(slug).join(number.to_string())
    }

    pub fn add_bytes(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub fn sub_bytes(&self, n: u64) {
        let _ = self.bytes.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            Some(v.saturating_sub(n))
        });
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn set_limit(&self, mb: u64) {
        self.limit_bytes.store(mb.saturating_mul(1_000_000), Ordering::Relaxed);
    }

    pub fn limit(&self) -> u64 {
        self.limit_bytes.load(Ordering::Relaxed)
    }

    pub fn over_limit(&self) -> bool {
        self.limit() > 0 && self.bytes() >= self.limit()
    }

    /// Purga los archivos más antiguos hasta quedar en el 80 % del límite.
    pub async fn enforce_limit(&self) -> AppResult<()> {
        let limit = self.limit();
        if limit == 0 {
            return Ok(());
        }
        let mut bytes = self.bytes();
        if bytes < limit {
            return Ok(());
        }
        let target = (limit as f64 * PURGE_TARGET_RATIO) as u64;

        let mut files: Vec<(PathBuf, u64)> = Vec::new();
        Self::collect(&self.dir, &mut files);
        files.sort_by_key(|(_, mtime)| *mtime);

        for (path, _) in files {
            if bytes <= target {
                break;
            }
            if let Ok(md) = std::fs::metadata(&path) {
                if std::fs::remove_file(&path).is_ok() {
                    let len = md.len();
                    bytes = bytes.saturating_sub(len);
                    self.sub_bytes(len);
                }
            }
        }
        Self::prune_empty(&self.dir);
        debug!(bytes, limit, "límite de caché aplicado (purge LRU)");
        Ok(())
    }

    /// Elimina el directorio de un episodio concreto.
    pub fn remove_episode(&self, slug: &str, number: i32) -> AppResult<()> {
        let dir = self.episode_dir(slug, number);
        if !dir.exists() {
            return Ok(());
        }
        let removed = Self::scan_bytes(&dir);
        std::fs::remove_dir_all(&dir)
            .map_err(|e| AppError::Cache(format!("eliminar {dir:?}: {e}")))?;
        self.sub_bytes(removed);
        let _ = std::fs::remove_dir(self.slug_dir(slug));
        Ok(())
    }

    /// Borra toda la caché y devuelve los bytes liberados.
    pub fn clear(&self) -> AppResult<u64> {
        let freed = self.bytes();
        for entry in std::fs::read_dir(&self.dir)
            .map_err(|e| AppError::Cache(format!("leer {:?}: {e}", self.dir)))?
        {
            if let Ok(entry) = entry {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        self.bytes.store(0, Ordering::Relaxed);
        debug!(freed, "caché de segmentos limpiada");
        Ok(freed)
    }

    fn scan_bytes(dir: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    total += Self::scan_bytes(&p);
                } else if let Ok(md) = e.metadata() {
                    total += md.len();
                }
            }
        }
        total
    }

    fn collect(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    Self::collect(&p, out);
                } else if let Ok(md) = e.metadata() {
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    out.push((p, mtime));
                }
            }
        }
    }

    fn prune_empty(dir: &Path) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    Self::prune_empty(&p);
                    let _ = std::fs::remove_dir(&p);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_bytes_across_write_remove() {
        let dir = std::env::temp_dir().join(format!("segcache-test-{}", std::process::id()));
        let cache = SegmentCache::new(&dir, "buffer").unwrap();
        cache.set_limit(1);

        let ep = cache.episode_dir("slug-demo", 1);
        std::fs::create_dir_all(&ep).unwrap();
        std::fs::write(ep.join("seg_0000.ts"), vec![0u8; 2_000_000]).unwrap();
        cache.add_bytes(2_000_000);

        assert_eq!(cache.bytes(), 2_000_000);
        assert!(cache.over_limit());

        cache.remove_episode("slug-demo", 1).unwrap();
        assert_eq!(cache.bytes(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_empties_everything() {
        let dir = std::env::temp_dir().join(format!("segcache-clear-{}", std::process::id()));
        let cache = SegmentCache::new(&dir, "buffer").unwrap();
        let ep = cache.episode_dir("a", 1);
        std::fs::create_dir_all(&ep).unwrap();
        std::fs::write(ep.join("seg_0000.ts"), vec![1u8; 50]).unwrap();
        cache.add_bytes(50);

        let freed = cache.clear().unwrap();
        assert_eq!(freed, 50);
        assert_eq!(cache.bytes(), 0);
        assert!(!ep.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enforce_limit_purges_oldest() {
        let dir = std::env::temp_dir().join(format!("segcache-lru-{}", std::process::id()));
        let cache = SegmentCache::new(&dir, "buffer").unwrap();
        cache.set_limit(1);

        for n in 1..=5 {
            let ep = cache.episode_dir("slug", n);
            std::fs::create_dir_all(&ep).unwrap();
            std::fs::write(ep.join("seg_0000.ts"), vec![0u8; 300_000]).unwrap();
            cache.add_bytes(300_000);
        }
        assert_eq!(cache.bytes(), 1_500_000);

        // ttl del test no existe; dar mtimes distintos
        let base = std::time::UNIX_EPOCH;
        let mut i = 0u64;
        for n in 1..=5 {
            let ep = cache.episode_dir("slug", n);
            let path = ep.join("seg_0000.ts");
            let t = base + std::time::Duration::from_secs(i * 60);
            if let Ok(f) = std::fs::File::open(&path) {
                let _ = f.set_times(std::fs::FileTimes::new().set_modified(t));
            }
            i += 1;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(cache.enforce_limit()).unwrap();
        assert!(cache.bytes() <= 800_000, "bytes={}", cache.bytes());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
