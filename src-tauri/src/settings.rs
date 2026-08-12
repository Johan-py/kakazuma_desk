use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::debug;

use crate::error::{AppError, AppResult};
use crate::infra::repos::SettingsRepo;
use crate::provider::{PROVIDER_JKANIME, ProviderRegistry};

/// Clave en la tabla `settings` donde se guarda la configuración del buffer.
pub const SETTINGS_KEY_SMART_BUFFER: &str = "smart_buffer";

/// Clave donde se guarda el proveedor por defecto.
pub const SETTINGS_KEY_PROVIDER: &str = "default_provider";

/// Configuración persistente del Smart Buffer.
///
/// Se serializa como JSON bajo `SETTINGS_KEY_SMART_BUFFER`.
/// Todos los rangos se sancionan en [`BufferConfig::sanitized`] antes de
/// persistir y de aplicar.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferConfig {
    /// Activa o desactiva el sistema completo.
    pub smart_buffer_enabled: bool,
    /// Nº de episodios futuros a precargar (0 = desactivado, máx 5).
    pub buffer_episode_count: u32,
    /// Porcentaje máximo de cada episodio a precargar (nunca 100 %).
    pub buffer_percentage: u32,
    /// Tamaño máximo de la caché en disco en MB.
    pub buffer_cache_limit_mb: u64,
    /// Límite de ancho de banda en Mbps para las descargas.
    pub buffer_bandwidth_limit_mbps: u64,
    /// Progreso (%) del episodio actual a partir del cual se inicia el buffer.
    pub buffer_trigger_percent: u32,
    /// Umbral de CPU (%) a partir del cual el buffer se pausa.
    pub buffer_cpu_threshold_percent: u32,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            smart_buffer_enabled: true,
            buffer_episode_count: 1,
            buffer_percentage: 20,
            buffer_cache_limit_mb: 1000,
            buffer_bandwidth_limit_mbps: 5,
            buffer_trigger_percent: 80,
            buffer_cpu_threshold_percent: 70,
        }
    }
}

impl BufferConfig {
    /// Acota todos los valores a rangos seguros.
    pub fn sanitized(mut self) -> Self {
        self.buffer_episode_count = self.buffer_episode_count.min(5);
        self.buffer_percentage = self.buffer_percentage.clamp(5, 90);
        self.buffer_cache_limit_mb = self.buffer_cache_limit_mb.clamp(100, 10_000);
        self.buffer_bandwidth_limit_mbps = self.buffer_bandwidth_limit_mbps.clamp(1, 100);
        self.buffer_trigger_percent = self.buffer_trigger_percent.clamp(50, 95);
        self.buffer_cpu_threshold_percent = self.buffer_cpu_threshold_percent.clamp(20, 100);
        self
    }
}

/// Servicio de configuración persistente con caché en memoria.
pub struct SettingsService {
    inner: Arc<RwLock<BufferConfig>>,
    provider: Arc<RwLock<String>>,
    pool: SqlitePool,
}

impl SettingsService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BufferConfig::default())),
            provider: Arc::new(RwLock::new(PROVIDER_JKANIME.to_string())),
            pool,
        }
    }

    /// Carga la configuración desde la base de datos. Si no existe, persiste
    /// los valores por defecto.
    pub async fn load(&self) -> AppResult<BufferConfig> {
        let cfg = match SettingsRepo::get(&self.pool, SETTINGS_KEY_SMART_BUFFER).await? {
            Some(raw) => serde_json::from_str::<BufferConfig>(&raw)
                .map(|c| c.sanitized())
                .unwrap_or_else(|e| {
                    debug!(error = %e, "config de buffer corrupta, usando defaults");
                    BufferConfig::default()
                }),
            None => {
                let defaults = BufferConfig::default();
                self.persist(&defaults).await?;
                defaults
            }
        };
        self.apply(&cfg);
        debug!(?cfg, "config de buffer cargada");

        self.load_provider().await?;
        Ok(cfg)
    }

    /// Carga el proveedor por defecto persistido (valida contra el registro).
    pub async fn load_provider(&self) -> AppResult<String> {
        let key = match SettingsRepo::get(&self.pool, SETTINGS_KEY_PROVIDER).await? {
            Some(k) if !k.trim().is_empty() => k,
            _ => PROVIDER_JKANIME.to_string(),
        };
        let key = if ProviderRegistry::valid_key(&key) {
            key
        } else {
            PROVIDER_JKANIME.to_string()
        };
        if let Ok(mut p) = self.provider.write() {
            *p = key.clone();
        }
        debug!(provider = %key, "proveedor por defecto cargado");
        Ok(key)
    }

    /// Proveedor por defecto actual (clave).
    pub fn provider_key(&self) -> String {
        self.provider
            .read()
            .map(|p| p.clone())
            .unwrap_or_else(|_| PROVIDER_JKANIME.to_string())
    }

    /// Persiste y aplica un nuevo proveedor por defecto.
    pub async fn set_provider_key(&self, key: &str) -> AppResult<String> {
        if !ProviderRegistry::valid_key(key) {
            return Err(AppError::Config(format!(
                "proveedor desconocido: {key}"
            )));
        }
        SettingsRepo::set(&self.pool, SETTINGS_KEY_PROVIDER, key).await?;
        if let Ok(mut p) = self.provider.write() {
            *p = key.to_string();
        }
        debug!(provider = %key, "proveedor por defecto actualizado");
        Ok(key.to_string())
    }

    /// Clona la configuración en memoria (rápido, sin IO).
    pub fn config(&self) -> BufferConfig {
        self.inner
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Valida, persiste y aplica una nueva configuración.
    pub async fn set_config(&self, cfg: BufferConfig) -> AppResult<BufferConfig> {
        let cfg = cfg.sanitized();
        self.persist(&cfg).await?;
        self.apply(&cfg);
        debug!(?cfg, "config de buffer actualizada");
        Ok(cfg)
    }

    async fn persist(&self, cfg: &BufferConfig) -> AppResult<()> {
        let raw = serde_json::to_string(cfg)
            .map_err(|e| AppError::Config(format!("serializar config: {e}")))?;
        SettingsRepo::set(&self.pool, SETTINGS_KEY_SMART_BUFFER, &raw).await
    }

    fn apply(&self, cfg: &BufferConfig) {
        if let Ok(mut g) = self.inner.write() {
            *g = cfg.clone();
        }
    }
}
