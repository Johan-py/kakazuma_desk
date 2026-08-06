use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Error de red: {0}")]
    Http(String),
    #[error("Error del proveedor: {0}")]
    Provider(String),
    #[error("Error de base de datos: {0}")]
    Db(String),
    #[error("Error de caché: {0}")]
    Cache(String),
    #[error("Error del reproductor: {0}")]
    Player(String),
    #[error("No encontrado: {0}")]
    NotFound(String),
    #[error("Configuración inválida: {0}")]
    Config(String),
    #[error("Operación no soportada por el proveedor: {0}")]
    Unsupported(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Http(_) => "http_error",
            AppError::Provider(_) => "provider_error",
            AppError::Db(_) => "db_error",
            AppError::Cache(_) => "cache_error",
            AppError::Player(_) => "player_error",
            AppError::NotFound(_) => "not_found",
            AppError::Config(_) => "config_error",
            AppError::Unsupported(_) => "unsupported",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}: {}", self.code(), self))
    }
}

pub type AppResult<T> = Result<T, AppError>;
