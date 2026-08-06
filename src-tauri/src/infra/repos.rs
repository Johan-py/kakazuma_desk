use sqlx::sqlite::SqlitePool;
use sqlx::Row;
use tracing::debug;

use crate::domain::{Anime, Episode, FavoriteEntry, Tag, WatchHistoryEntry};
use crate::error::{AppError, AppResult};

pub struct AnimeRepo;

impl AnimeRepo {
    /// Inserta o actualiza un anime por slug y devuelve su id local.
    pub async fn upsert(pool: &SqlitePool, a: &Anime) -> AppResult<i64> {
        let row = sqlx::query(
            r#"
            INSERT INTO anime (slug, name, synopsis, season, status, cover_image, total_episodes, anime_type, url, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch())
            ON CONFLICT(slug) DO UPDATE SET
                name = excluded.name,
                synopsis = excluded.synopsis,
                season = excluded.season,
                status = excluded.status,
                cover_image = excluded.cover_image,
                total_episodes = excluded.total_episodes,
                anime_type = excluded.anime_type,
                url = excluded.url,
                updated_at = unixepoch()
            RETURNING id_anime
            "#,
        )
        .bind(&a.slug)
        .bind(&a.name)
        .bind(a.synopsis.as_deref())
        .bind(a.season.as_deref())
        .bind(a.status.as_deref())
        .bind(a.cover_image.as_deref())
        .bind(a.total_episodes)
        .bind(a.anime_type.as_deref())
        .bind(&a.url)
        .fetch_one(pool)
        .await
        .map_err(db_err)?;

        let id = row.get::<i64, _>(0);
        Ok(id)
    }

    pub async fn get_by_slug(pool: &SqlitePool, slug: &str) -> AppResult<Option<Anime>> {
        let row = sqlx::query(
            r#"
            SELECT a.id_anime, a.slug, a.name, a.synopsis, a.season, a.status,
                   a.cover_image, a.total_episodes, a.anime_type, a.url,
                   COALESCE((SELECT GROUP_CONCAT(t.name, ',')
                              FROM anime_tag at JOIN tag t ON t.id_tag = at.id_tag
                              WHERE at.id_anime = a.id_anime), '') AS genres
            FROM anime a WHERE a.slug = ?1
            "#,
        )
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?;

        row.as_ref().map(from_anime_row).transpose()
    }

    pub async fn get_by_id(pool: &SqlitePool, id: i64) -> AppResult<Option<Anime>> {
        let row = sqlx::query(
            r#"
            SELECT a.id_anime, a.slug, a.name, a.synopsis, a.season, a.status,
                   a.cover_image, a.total_episodes, a.anime_type, a.url,
                   COALESCE((SELECT GROUP_CONCAT(t.name, ',')
                              FROM anime_tag at JOIN tag t ON t.id_tag = at.id_tag
                              WHERE at.id_anime = a.id_anime), '') AS genres
            FROM anime a WHERE a.id_anime = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?;

        row.as_ref().map(from_anime_row).transpose()
    }

    pub async fn episodes(pool: &SqlitePool, anime_id: i64) -> AppResult<Vec<Episode>> {
        let rows = sqlx::query(
            "SELECT id_episode, id_anime, number, title, video_url, duration FROM episode WHERE id_anime = ?1 ORDER BY number ASC",
        )
        .bind(anime_id)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;

        rows.iter().map(from_episode_row).collect()
    }
}

pub struct EpisodeRepo;

impl EpisodeRepo {
    pub async fn upsert_many(pool: &SqlitePool, anime_id: i64, episodes: &[Episode]) -> AppResult<()> {
        let mut tx = pool.begin().await.map_err(db_err)?;
        for ep in episodes {
            sqlx::query(
                r#"
                INSERT INTO episode (id_anime, number, title, video_url, duration)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(id_anime, number) DO UPDATE SET title = excluded.title
                "#,
            )
            .bind(anime_id)
            .bind(ep.number)
            .bind(ep.title.as_deref())
            .bind(ep.video_url.as_deref())
            .bind(ep.duration)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        debug!(anime_id, count = episodes.len(), "episodios sincronizados");
        Ok(())
    }

    pub async fn set_video_url(pool: &SqlitePool, anime_id: i64, number: i32, url: &str) -> AppResult<()> {
        sqlx::query("UPDATE episode SET video_url = ?1 WHERE id_anime = ?2 AND number = ?3")
            .bind(url)
            .bind(anime_id)
            .bind(number)
            .execute(pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

pub struct TagRepo;

impl TagRepo {
    /// Asegura que el tag exista y devuelve su id.
    pub async fn ensure(pool: &SqlitePool, name: &str) -> AppResult<i64> {
        let row = sqlx::query("SELECT id_tag FROM tag WHERE name = ?1")
            .bind(name)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;

        if let Some(r) = row {
            return Ok(r.get::<i64, _>(0));
        }

        let row = sqlx::query("INSERT INTO tag (name) VALUES (?1) RETURNING id_tag")
            .bind(name)
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
        Ok(row.get::<i64, _>(0))
    }

    /// Reemplaza los tags de un anime.
    pub async fn set_for_anime(pool: &SqlitePool, anime_id: i64, tags: &[String]) -> AppResult<()> {
        sqlx::query("DELETE FROM anime_tag WHERE id_anime = ?1")
            .bind(anime_id)
            .execute(pool)
            .await
            .map_err(db_err)?;

        for name in tags {
            let tag_id = Self::ensure(pool, name).await?;
            sqlx::query("INSERT OR IGNORE INTO anime_tag (id_anime, id_tag) VALUES (?1, ?2)")
                .bind(anime_id)
                .bind(tag_id)
                .execute(pool)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }

    pub async fn all(pool: &SqlitePool) -> AppResult<Vec<Tag>> {
        let rows = sqlx::query("SELECT id_tag, name, description FROM tag ORDER BY name ASC")
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                Ok(Tag {
                    id: r.get::<i64, _>("id_tag"),
                    name: r.get::<String, _>("name"),
                    description: r.get::<Option<String>, _>("description"),
                })
            })
            .collect()
    }
}

pub struct HistoryRepo;

impl HistoryRepo {
    /// Guarda/actualiza el progreso de un episodio de un anime.
    pub async fn upsert(
        pool: &SqlitePool,
        anime_id: i64,
        episode_id: Option<i64>,
        position: f64,
        duration: f64,
    ) -> AppResult<()> {
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            r#"
            UPDATE watch_history
            SET id_episode = ?2, playback_position = ?3, duration = ?4, date_last_view = ?5
            WHERE id_anime = ?1 AND id_episode = ?2
            "#,
        )
        .bind(anime_id)
        .bind(episode_id)
        .bind(position)
        .bind(duration)
        .bind(now)
        .execute(pool)
        .await
        .map_err(db_err)?;

        if updated.rows_affected() == 0 {
            sqlx::query(
                r#"
                INSERT INTO watch_history (id_anime, id_episode, playback_position, duration, date_first_view, date_last_view)
                VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                "#,
            )
            .bind(anime_id)
            .bind(episode_id)
            .bind(position)
            .bind(duration)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    /// Último episodio visto por anime, ordenado por fecha.
    pub async fn continue_watching(pool: &SqlitePool) -> AppResult<Vec<WatchHistoryEntry>> {
        let rows = sqlx::query(
            r#"
            SELECT h.id_history, h.playback_position, h.duration, h.date_first_view, h.date_last_view,
                   a.id_anime, a.slug, a.name, a.synopsis, a.season, a.status, a.cover_image,
                   a.total_episodes, a.anime_type, a.url,
                   e.id_episode, e.number, e.title, e.video_url, e.duration AS ep_duration
            FROM watch_history h
            JOIN anime a ON a.id_anime = h.id_anime
            LEFT JOIN episode e ON e.id_episode = h.id_episode
            WHERE h.id_history = (
                SELECT h2.id_history FROM watch_history h2
                WHERE h2.id_anime = h.id_anime
                ORDER BY h2.date_last_view DESC, h2.id_history DESC LIMIT 1
            )
            ORDER BY h.date_last_view DESC
            "#,
        )
        .fetch_all(pool)
        .await
        .map_err(db_err)?;

        rows.iter().map(from_history_row).collect()
    }

    pub async fn clear(pool: &SqlitePool) -> AppResult<()> {
        sqlx::query("DELETE FROM watch_history").execute(pool).await.map_err(db_err)?;
        Ok(())
    }
}

pub struct FavoriteRepo;

impl FavoriteRepo {
    pub async fn add(pool: &SqlitePool, anime_id: i64) -> AppResult<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR IGNORE INTO favorite_anime (id_anime, date_added) VALUES (?1, ?2)",
        )
        .bind(anime_id)
        .bind(now)
        .execute(pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub async fn remove(pool: &SqlitePool, anime_id: i64) -> AppResult<()> {
        sqlx::query("DELETE FROM favorite_anime WHERE id_anime = ?1")
            .bind(anime_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn is_favorite(pool: &SqlitePool, anime_id: i64) -> AppResult<bool> {
        let row = sqlx::query("SELECT 1 FROM favorite_anime WHERE id_anime = ?1")
            .bind(anime_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
        Ok(row.is_some())
    }

    pub async fn list(pool: &SqlitePool) -> AppResult<Vec<FavoriteEntry>> {
        let rows = sqlx::query(
            r#"
            SELECT f.id_favorite, f.date_added,
                   a.id_anime, a.slug, a.name, a.synopsis, a.season, a.status, a.cover_image,
                   a.total_episodes, a.anime_type, a.url
            FROM favorite_anime f
            JOIN anime a ON a.id_anime = f.id_anime
            ORDER BY f.date_added DESC
            "#,
        )
        .fetch_all(pool)
        .await
        .map_err(db_err)?;

        rows.iter().map(from_favorite_row).collect()
    }
}

// ---------- utilidades de mapeo ----------

fn db_err(e: sqlx::Error) -> AppError {
    AppError::Db(format!("{e}"))
}

fn from_anime_row(r: &sqlx::sqlite::SqliteRow) -> AppResult<Anime> {
    let genres: String = r.try_get("genres").unwrap_or_default();
    let genres = if genres.is_empty() {
        Vec::new()
    } else {
        genres.split(',').map(|s| s.trim().to_string()).collect()
    };
    Ok(Anime {
        id: r.get("id_anime"),
        slug: r.get("slug"),
        name: r.get("name"),
        synopsis: r.get("synopsis"),
        season: r.get("season"),
        status: r.get("status"),
        cover_image: r.get("cover_image"),
        total_episodes: r.get("total_episodes"),
        anime_type: r.get("anime_type"),
        url: r.get("url"),
        genres,
    })
}

fn from_episode_row(r: &sqlx::sqlite::SqliteRow) -> AppResult<Episode> {
    Ok(Episode {
        id: r.get("id_episode"),
        anime_id: r.get("id_anime"),
        number: r.get("number"),
        title: r.get("title"),
        video_url: r.get("video_url"),
        duration: r.get("duration"),
    })
}

fn from_history_row(r: &sqlx::sqlite::SqliteRow) -> AppResult<WatchHistoryEntry> {
    Ok(WatchHistoryEntry {
        id: r.get("id_history"),
        anime: Anime {
            id: r.get("id_anime"),
            slug: r.get("slug"),
            name: r.get("name"),
            synopsis: r.get("synopsis"),
            season: r.get("season"),
            status: r.get("status"),
            cover_image: r.get("cover_image"),
            total_episodes: r.get("total_episodes"),
            anime_type: r.get("anime_type"),
            url: r.get("url"),
            genres: Vec::new(),
        },
        episode: r
            .try_get::<Option<i64>, _>("id_episode")
            .ok()
            .flatten()
            .map(|id| Episode {
                id,
                anime_id: r.get("id_anime"),
                number: r.try_get("number").ok().unwrap_or_default(),
                title: r.try_get("title").ok(),
                video_url: r.try_get("video_url").ok(),
                duration: r.try_get("ep_duration").ok(),
            }),
        playback_position: r.get("playback_position"),
        duration: r.get("duration"),
        date_first_view: r.get("date_first_view"),
        date_last_view: r.get("date_last_view"),
    })
}

fn from_favorite_row(r: &sqlx::sqlite::SqliteRow) -> AppResult<FavoriteEntry> {
    Ok(FavoriteEntry {
        id: r.get("id_favorite"),
        anime: Anime {
            id: r.get("id_anime"),
            slug: r.get("slug"),
            name: r.get("name"),
            synopsis: r.get("synopsis"),
            season: r.get("season"),
            status: r.get("status"),
            cover_image: r.get("cover_image"),
            total_episodes: r.get("total_episodes"),
            anime_type: r.get("anime_type"),
            url: r.get("url"),
            genres: Vec::new(),
        },
        date_added: r.get("date_added"),
    })
}
