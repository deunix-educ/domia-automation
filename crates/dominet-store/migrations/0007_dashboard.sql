-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Tableaux de bord et widgets (R2, ADR 0018).
--
-- `device_id`, `scene_id` et `camera_uuid` n'ont PAS de cle etrangere : les tables `device` (R3), `scene` (R5) et `camera`
-- (R6) n'existent pas encore. Leurs migrations ajouteront la contrainte (et mettront a NULL les references orphelines).

-- Permet une contrainte d'exclusion mixte (egalite d'un entier + chevauchement d'un rectangle).
CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE dashboard (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name       TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    owner_id   BIGINT  NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    is_default BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX dashboard_owner_idx ON dashboard (owner_id, name);
-- Au plus un tableau par defaut pour chaque proprietaire.
CREATE UNIQUE INDEX dashboard_one_default_per_owner ON dashboard (owner_id) WHERE is_default;

CREATE TABLE widget (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    dashboard_id BIGINT   NOT NULL REFERENCES dashboard (id) ON DELETE CASCADE,
    widget_type  TEXT     NOT NULL CHECK (widget_type IN ('camera', 'device', 'scene', 'grafana', 'text', 'grille')),
    title        TEXT     NOT NULL DEFAULT '' CHECK (length(title) <= 64),
    device_id    BIGINT   CHECK (device_id > 0),
    scene_id     BIGINT   CHECK (scene_id > 0),
    camera_uuid  UUID,
    -- Modele d'affichage ('' = automatique selon le type) ; la coherence avec widget_type est verifiee par le domaine.
    presentation TEXT     NOT NULL DEFAULT '' CHECK (length(presentation) <= 32),
    config       JSONB    NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(config) = 'object' AND pg_column_size(config) <= 4096),
    -- HTML du widget `text` (assaini par le serveur a l'enregistrement).
    content      TEXT     NOT NULL DEFAULT '' CHECK (length(content) <= 20000),
    -- Grille de 12 colonnes ; les lignes sont illimitees (jusqu'a 999).
    grid_col     SMALLINT NOT NULL DEFAULT 0 CHECK (grid_col BETWEEN 0 AND 11),
    grid_row     SMALLINT NOT NULL DEFAULT 0 CHECK (grid_row BETWEEN 0 AND 999),
    width        SMALLINT NOT NULL DEFAULT 2 CHECK (width BETWEEN 1 AND 12),
    height       SMALLINT NOT NULL DEFAULT 2 CHECK (height BETWEEN 1 AND 12),
    CONSTRAINT widget_inside_grid CHECK (grid_col + width <= 12),
    -- Deux widgets d'un meme tableau ne se chevauchent jamais : garanti par la base, donc meme si deux deplacements
    -- arrivent en meme temps. Les rectangles sont en coordonnees de cellules, bornes incluses ; des widgets contigus passent.
    CONSTRAINT widget_no_overlap EXCLUDE USING gist (
        dashboard_id WITH =,
        box(point(grid_col, grid_row), point(grid_col + width - 1, grid_row + height - 1)) WITH &&
    )
);
CREATE INDEX widget_dashboard_idx ON widget (dashboard_id, grid_row, grid_col);
