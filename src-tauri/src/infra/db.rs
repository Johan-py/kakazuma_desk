use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use tracing::{debug, info};

use crate::error::{AppError, AppResult};

/// Piscina de conexiones SQLite con WAL y foreign keys habilitadas.
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(path: &Path) -> AppResult<Self> {
        let db_path = path.to_str().ok_or_else(|| AppError::Config("ruta de DB inválida".into()))?;

        let options = SqliteConnectOptions::from_str(db_path)
            .map_err(|e| AppError::Config(format!("opciones SQLite: {e}")))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .auto_vacuum(sqlx::sqlite::SqliteAutoVacuum::Incremental);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(|e| AppError::Db(format!("conexión SQLite: {e}")))?;

        info!(db = %db_path, "base de datos conectada (WAL)");
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> AppResult<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| AppError::Db(format!("migraciones: {e}")))?;
        debug!("migraciones aplicadas");
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
