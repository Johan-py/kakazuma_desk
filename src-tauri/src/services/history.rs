use sqlx::SqlitePool;

use crate::domain::WatchHistoryEntry;
use crate::error::AppResult;
use crate::infra::repos::{AnimeRepo, HistoryRepo};

pub struct HistoryService {
    pool: SqlitePool,
}

impl HistoryService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Guarda el progreso de reproducción de un episodio.
    pub async fn save_progress(
        &self,
        anime_slug: &str,
        episode_number: Option<i32>,
        position: f64,
        duration: f64,
    ) -> AppResult<()> {
        let anime = match AnimeRepo::get_by_slug(&self.pool, anime_slug).await? {
            Some(a) => a,
            None => {
                // el anime aún no está persistido; no hay nada que guardar
                return Ok(());
            }
        };

        let episode_id = match episode_number {
            Some(n) => {
                let eps = AnimeRepo::episodes(&self.pool, anime.id).await?;
                eps.iter().find(|e| e.number == n).map(|e| e.id)
            }
            None => None,
        };

        HistoryRepo::upsert(&self.pool, anime.id, episode_id, position, duration).await
    }

    /// Lista de "Continuar viendo" (último episodio por anime).
    pub async fn continue_watching(&self) -> AppResult<Vec<WatchHistoryEntry>> {
        HistoryRepo::continue_watching(&self.pool).await
    }

    pub async fn clear(&self) -> AppResult<()> {
        HistoryRepo::clear(&self.pool).await
    }
}

