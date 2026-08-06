use sqlx::SqlitePool;

use crate::domain::FavoriteEntry;
use crate::error::AppResult;
use crate::infra::repos::{AnimeRepo, FavoriteRepo};

pub struct FavoriteService {
    pool: SqlitePool,
}

impl FavoriteService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn add(&self, anime_slug: &str) -> AppResult<bool> {
        let anime = AnimeRepo::get_by_slug(&self.pool, anime_slug).await?;
        let Some(anime) = anime else { return Ok(false) };
        FavoriteRepo::add(&self.pool, anime.id).await?;
        Ok(true)
    }

    pub async fn remove(&self, anime_slug: &str) -> AppResult<()> {
        let anime = AnimeRepo::get_by_slug(&self.pool, anime_slug).await?;
        if let Some(anime) = anime {
            FavoriteRepo::remove(&self.pool, anime.id).await?;
        }
        Ok(())
    }

    pub async fn is_favorite(&self, anime_slug: &str) -> AppResult<bool> {
        let anime = AnimeRepo::get_by_slug(&self.pool, anime_slug).await?;
        match anime {
            Some(a) => FavoriteRepo::is_favorite(&self.pool, a.id).await,
            None => Ok(false),
        }
    }

    pub async fn list(&self) -> AppResult<Vec<FavoriteEntry>> {
        FavoriteRepo::list(&self.pool).await
    }
}
