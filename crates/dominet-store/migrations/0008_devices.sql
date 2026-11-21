-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Appareils, historique, sources MQTT et stations meteo (R3, ADR 0025).
--
-- Relie aussi `widget.device_id` a `device` (cle etrangere annoncee par la migration 0007, ADR 0018) : les references
-- orphelines sont mises a NULL avant d'ajouter la contrainte, puis `ON DELETE SET NULL` comme en Python.

CREATE TABLE device_group (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name        TEXT   NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    location_id BIGINT REFERENCES location (id) ON DELETE SET NULL,
    description TEXT   NOT NULL DEFAULT '' CHECK (length(description) <= 256),
    icon        TEXT   NOT NULL DEFAULT '' CHECK (length(icon) <= 32)
);
CREATE INDEX device_group_name_idx ON device_group (name);

-- Referentiel global : cle d'etat (temperature, battery...) -> nom affiche, icone Lucide, unite.
CREATE TABLE device_label (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    key       TEXT NOT NULL UNIQUE CHECK (length(key) BETWEEN 1 AND 64),
    name      TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    icon_name TEXT NOT NULL CHECK (length(icon_name) BETWEEN 1 AND 64),
    unit      TEXT NOT NULL DEFAULT '' CHECK (length(unit) <= 32)
);

CREATE TABLE device (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- = topic Zigbee2MQTT (peut contenir des '/') ; unique comme en Python (sensible a la casse).
    friendly_name TEXT    NOT NULL UNIQUE CHECK (length(friendly_name) BETWEEN 1 AND 128),
    ieee          TEXT    NOT NULL DEFAULT '' CHECK (length(ieee) <= 64),
    vendor        TEXT    NOT NULL DEFAULT '' CHECK (length(vendor) <= 64),
    model_id      TEXT    NOT NULL DEFAULT '' CHECK (length(model_id) <= 64),
    description   TEXT    NOT NULL DEFAULT '' CHECK (length(description) <= 256),
    device_type   TEXT    NOT NULL DEFAULT 'generic'
        CHECK (device_type IN ('sensor', 'binary_sensor', 'switch', 'light', 'cover', 'climate', 'remote', 'generic')),
    group_id      BIGINT  REFERENCES device_group (id) ON DELETE SET NULL,
    location_id   BIGINT  REFERENCES location (id) ON DELETE SET NULL,
    -- Topic complet de l'etat = {base_topic}/{friendly_name} ; commande = ... + '/set'.
    base_topic    TEXT    NOT NULL DEFAULT 'zigbee2mqtt' CHECK (length(base_topic) <= 128),
    -- Etat courant (REMPLACE a chaque message), mis a jour par l'ecoute MQTT.
    state         JSONB   NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(state) = 'object'),
    last_seen     TIMESTAMPTZ,
    available     BOOLEAN NOT NULL DEFAULT FALSE,
    exposes       JSONB   NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(exposes) = 'array'),
    active        BOOLEAN NOT NULL DEFAULT TRUE,
    icon          TEXT    NOT NULL DEFAULT '' CHECK (length(icon) <= 32),
    image_url     TEXT    NOT NULL DEFAULT '' CHECK (length(image_url) <= 512),
    -- Cles d'etat historisees / masquees : listes de textes (et non JSON : le type garantit la forme).
    tracked_keys  TEXT[]  NOT NULL DEFAULT '{}',
    hidden_keys   TEXT[]  NOT NULL DEFAULT '{}',
    grafana_dashboard BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX device_group_idx ON device (group_id);
CREATE INDEX device_location_idx ON device (location_id);

-- Image televersee d'un appareil (JPEG reduit par le serveur) : en base, sans volume a sauvegarder a part (ADR 0025).
CREATE TABLE device_image (
    device_id    BIGINT PRIMARY KEY REFERENCES device (id) ON DELETE CASCADE,
    content_type TEXT        NOT NULL CHECK (content_type IN ('image/jpeg', 'image/png', 'image/webp', 'image/gif')),
    data         BYTEA       NOT NULL CHECK (length(data) BETWEEN 1 AND 5242880),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Historique des cles suivies : meme forme de colonnes qu'en Python (les tableaux Grafana n'ont qu'un nom de table a changer),
-- une ligne par appareil, cle et seconde.
CREATE TABLE device_state_log (
    device_id  BIGINT           NOT NULL REFERENCES device (id) ON DELETE CASCADE,
    key        TEXT             NOT NULL CHECK (length(key) BETWEEN 1 AND 64),
    ts         TIMESTAMPTZ      NOT NULL,
    value_num  DOUBLE PRECISION,
    value_str  TEXT             NOT NULL DEFAULT '' CHECK (length(value_str) <= 128),
    CONSTRAINT device_state_log_unique UNIQUE (device_id, key, ts)
);
-- Lecture d'un graphique (appareil, cle, du plus recent au plus ancien) et purge par date.
CREATE INDEX device_state_log_recent_idx ON device_state_log (device_id, key, ts DESC);
CREATE INDEX device_state_log_ts_idx ON device_state_log (ts);

-- Abonnements MQTT supplementaires (hors Zigbee2MQTT) : Linky, SolarMax, meteo...
CREATE TABLE mqtt_source (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name             TEXT     NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    topic_pattern    TEXT     NOT NULL CHECK (length(topic_pattern) BETWEEN 1 AND 255),
    qos              SMALLINT NOT NULL DEFAULT 0 CHECK (qos BETWEEN 0 AND 2),
    active           BOOLEAN  NOT NULL DEFAULT TRUE,
    log_messages     BOOLEAN  NOT NULL DEFAULT FALSE,
    auto_device      BOOLEAN  NOT NULL DEFAULT FALSE,
    auto_device_type TEXT     NOT NULL DEFAULT 'sensor'
        CHECK (auto_device_type IN ('', 'sensor', 'binary_sensor', 'switch', 'light', 'cover', 'climate', 'remote', 'generic')),
    base_topic       TEXT     NOT NULL DEFAULT '' CHECK (length(base_topic) <= 128),
    -- Code Python de la version precedente : conserve et affiche, execute a partir de R4 (script Rhai).
    handler_code     TEXT     NOT NULL DEFAULT '' CHECK (length(handler_code) <= 20000),
    description      TEXT     NOT NULL DEFAULT '' CHECK (length(description) <= 256)
);
CREATE INDEX mqtt_source_name_idx ON mqtt_source (name);

CREATE TABLE weather_station (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name              TEXT             NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    latitude          DOUBLE PRECISION NOT NULL CHECK (latitude BETWEEN -90 AND 90),
    longitude         DOUBLE PRECISION NOT NULL CHECK (longitude BETWEEN -180 AND 180),
    altitude          INTEGER          NOT NULL DEFAULT 0 CHECK (altitude BETWEEN -500 AND 9000),
    timezone          TEXT             NOT NULL DEFAULT 'Europe/Paris' CHECK (length(timezone) BETWEEN 1 AND 50),
    -- Topic publie : meteo/<suffixe>.
    mqtt_topic_suffix TEXT             NOT NULL DEFAULT 'meteo_locale' CHECK (mqtt_topic_suffix ~ '^[A-Za-z0-9_.-]{1,100}$'),
    refresh_minutes   SMALLINT         NOT NULL DEFAULT 10 CHECK (refresh_minutes BETWEEN 1 AND 1440),
    enabled           BOOLEAN          NOT NULL DEFAULT TRUE,
    description       TEXT             NOT NULL DEFAULT '' CHECK (length(description) <= 255)
);

-- Cle etrangere annoncee par la migration 0007.
UPDATE widget SET device_id = NULL WHERE device_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM device d WHERE d.id = widget.device_id);
ALTER TABLE widget ADD CONSTRAINT widget_device_fk FOREIGN KEY (device_id) REFERENCES device (id) ON DELETE SET NULL;
