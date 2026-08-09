-- Migración 0002: configuración persistente (key-value).
-- El Smart Buffer guarda su configuración bajo la clave "smart_buffer" (JSON).

CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
