-- Modelo Entidad Relación - Kakasuma Desktop
-- SQLite con WAL habilitado desde el pool de conexiones.

CREATE TABLE IF NOT EXISTS anime (
    id_anime        INTEGER PRIMARY KEY AUTOINCREMENT,
    slug            TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    synopsis        TEXT,
    season          TEXT,
    status          TEXT,
    cover_image     TEXT,
    total_episodes  INTEGER DEFAULT 0,
    anime_type      TEXT,
    url             TEXT NOT NULL,
    updated_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS episode (
    id_episode   INTEGER PRIMARY KEY AUTOINCREMENT,
    id_anime     INTEGER NOT NULL REFERENCES anime(id_anime) ON DELETE CASCADE,
    number       INTEGER NOT NULL,
    title        TEXT,
    video_url    TEXT,
    duration     REAL,
    UNIQUE (id_anime, number)
);

CREATE TABLE IF NOT EXISTS tag (
    id_tag      INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL UNIQUE,
    description TEXT
);

CREATE TABLE IF NOT EXISTS anime_tag (
    id_anime INTEGER NOT NULL REFERENCES anime(id_anime) ON DELETE CASCADE,
    id_tag   INTEGER NOT NULL REFERENCES tag(id_tag) ON DELETE CASCADE,
    PRIMARY KEY (id_anime, id_tag)
);

CREATE TABLE IF NOT EXISTS watch_history (
    id_history        INTEGER PRIMARY KEY AUTOINCREMENT,
    id_anime          INTEGER NOT NULL REFERENCES anime(id_anime) ON DELETE CASCADE,
    id_episode        INTEGER REFERENCES episode(id_episode) ON DELETE CASCADE,
    playback_position REAL NOT NULL DEFAULT 0,
    duration          REAL NOT NULL DEFAULT 0,
    date_first_view   INTEGER NOT NULL,
    date_last_view    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_watch_history_anime ON watch_history(id_anime);
CREATE INDEX IF NOT EXISTS idx_watch_history_last_view ON watch_history(date_last_view DESC);

CREATE TABLE IF NOT EXISTS favorite_anime (
    id_favorite INTEGER PRIMARY KEY AUTOINCREMENT,
    id_anime    INTEGER NOT NULL UNIQUE REFERENCES anime(id_anime) ON DELETE CASCADE,
    date_added  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_favorite_added ON favorite_anime(date_added DESC);
