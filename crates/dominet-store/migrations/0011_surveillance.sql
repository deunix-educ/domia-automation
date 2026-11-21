-- SPDX-License-Identifier: AGPL-3.0-or-later
-- R6 lot 1 (ADR 0043) : cameras, presets PTZ, enregistrements, snapshots, mouvements, detections, configuration du detecteur.

-- `uuid` (32 caracteres hexadecimaux en texte) est le nom du path MediaMTX et la cle publique (widget `camera_uuid`, sujet `camera:<uuid>`).
-- Les mots de passe et la cle d'API sont stockes **chiffres** (`v1:...`, voir `SecretBox`) ; chaine vide = non renseigne.
CREATE TABLE camera (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    uuid                UUID        NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    name                TEXT        NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    location_id         BIGINT REFERENCES location (id) ON DELETE SET NULL,
    active              BOOLEAN     NOT NULL DEFAULT TRUE,
    priority            INTEGER     NOT NULL DEFAULT 1 CHECK (priority BETWEEN 1 AND 9),
    last_seen           TIMESTAMPTZ,
    notes               TEXT        NOT NULL DEFAULT '',
    -- flux source
    proto               TEXT        NOT NULL DEFAULT 'rtsp' CHECK (proto IN ('rtsp', 'rtmp')),
    ip                  TEXT        NOT NULL DEFAULT '' CHECK (length(ip) <= 64),
    port                INTEGER     NOT NULL DEFAULT 554 CHECK (port BETWEEN 1 AND 65535),
    path                TEXT        NOT NULL DEFAULT '' CHECK (length(path) <= 128),
    rtsp_user           TEXT        NOT NULL DEFAULT '' CHECK (length(rtsp_user) <= 128),
    rtsp_pass_enc       TEXT        NOT NULL DEFAULT '',
    fps                 INTEGER     NOT NULL DEFAULT 10 CHECK (fps BETWEEN 1 AND 60),
    rtsp_any_port       BOOLEAN     NOT NULL DEFAULT FALSE,
    rtsp_transport_tcp  BOOLEAN     NOT NULL DEFAULT FALSE,
    recording_mode      TEXT        NOT NULL DEFAULT 'none' CHECK (recording_mode IN ('none', 'continuous', 'motion', 'manual')),
    -- jour / nuit
    night_mode_api      TEXT        NOT NULL DEFAULT 'none' CHECK (night_mode_api IN ('none', 'onvif', 'openipc', 'thingino', 'goform')),
    web_port            INTEGER     NOT NULL DEFAULT 80 CHECK (web_port BETWEEN 1 AND 65535),
    web_api_key_enc     TEXT        NOT NULL DEFAULT '',
    web_user            TEXT        NOT NULL DEFAULT '' CHECK (length(web_user) <= 128),
    web_pass_enc        TEXT        NOT NULL DEFAULT '',
    -- ONVIF
    onvif               BOOLEAN     NOT NULL DEFAULT FALSE,
    onvif_port          INTEGER     NOT NULL DEFAULT 80 CHECK (onvif_port BETWEEN 1 AND 65535),
    onvif_user          TEXT        NOT NULL DEFAULT '' CHECK (length(onvif_user) <= 128),
    onvif_pass_enc      TEXT        NOT NULL DEFAULT '',
    main_token          TEXT        NOT NULL DEFAULT '' CHECK (length(main_token) <= 64),
    ptz                 BOOLEAN     NOT NULL DEFAULT FALSE,
    -- YOLO
    yolo_enabled        BOOLEAN     NOT NULL DEFAULT FALSE,
    yolo_classes        TEXT[]      NOT NULL DEFAULT '{}',
    yolo_min_confidence DOUBLE PRECISION NOT NULL DEFAULT 0.5 CHECK (yolo_min_confidence BETWEEN 0 AND 1),
    -- renseigne par la synchronisation ONVIF
    manufacturer        TEXT        NOT NULL DEFAULT '' CHECK (length(manufacturer) <= 100),
    model               TEXT        NOT NULL DEFAULT '' CHECK (length(model) <= 100),
    firmware            TEXT        NOT NULL DEFAULT '' CHECK (length(firmware) <= 100),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX camera_order_idx ON camera (priority, name);

CREATE TABLE ptz_preset (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    camera_id BIGINT NOT NULL REFERENCES camera (id) ON DELETE CASCADE,
    name      TEXT   NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    token     TEXT   NOT NULL DEFAULT '' CHECK (length(token) <= 64),
    pan       DOUBLE PRECISION NOT NULL DEFAULT 0,
    tilt      DOUBLE PRECISION NOT NULL DEFAULT 0,
    zoom      DOUBLE PRECISION NOT NULL DEFAULT 0,
    position  SMALLINT NOT NULL DEFAULT 0 CHECK (position >= 0),
    UNIQUE (camera_id, name)
);

CREATE TABLE recording (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    camera_id    BIGINT      NOT NULL REFERENCES camera (id) ON DELETE CASCADE,
    started_at   TIMESTAMPTZ NOT NULL,
    duration_ms  BIGINT CHECK (duration_ms >= 0),
    segment_path TEXT        NOT NULL UNIQUE CHECK (length(segment_path) BETWEEN 1 AND 512),
    reduct_ts    BIGINT,
    event_type   TEXT        NOT NULL DEFAULT 'continuous' CHECK (event_type IN ('continuous', 'motion', 'manual')),
    status       TEXT        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'uploading', 'done', 'error')),
    error        TEXT        NOT NULL DEFAULT '',
    size_bytes   BIGINT CHECK (size_bytes >= 0)
);
CREATE INDEX recording_camera_started_idx ON recording (camera_id, started_at DESC);
CREATE INDEX recording_status_idx ON recording (status) WHERE status <> 'done';

CREATE TABLE snapshot (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    camera_id  BIGINT      NOT NULL REFERENCES camera (id) ON DELETE CASCADE,
    taken_at   TIMESTAMPTZ NOT NULL,
    -- Chemin relatif au dossier des snapshots (`<uuid>/<horodatage>.jpg`).
    file       TEXT        NOT NULL CHECK (length(file) BETWEEN 1 AND 512),
    size_bytes INTEGER     NOT NULL DEFAULT 0 CHECK (size_bytes >= 0)
);
CREATE INDEX snapshot_camera_taken_idx ON snapshot (camera_id, taken_at DESC);

CREATE TABLE motion_event (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    camera_id   BIGINT      NOT NULL REFERENCES camera (id) ON DELETE CASCADE,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Rapport de pixels modifies, 0..1.
    confidence  DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (confidence BETWEEN 0 AND 1)
);
CREATE INDEX motion_event_camera_idx ON motion_event (camera_id, detected_at DESC);

CREATE TABLE detection_event (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    camera_id    BIGINT      NOT NULL REFERENCES camera (id) ON DELETE CASCADE,
    detected_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    triggered_by TEXT        NOT NULL DEFAULT 'motion' CHECK (triggered_by IN ('motion', 'periodic')),
    -- [{"class":"person","confidence":0.87,"bbox":[x1,y1,x2,y2]}, ...]
    labels       JSONB       NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(labels) = 'array'),
    frame_path   TEXT        NOT NULL DEFAULT '' CHECK (length(frame_path) <= 512)
);
CREATE INDEX detection_event_camera_idx ON detection_event (camera_id, detected_at DESC);

-- Singleton : une seule ligne (`id = 1`).
CREATE TABLE detector_config (
    id          SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    backend     TEXT        NOT NULL DEFAULT 'yolov8n' CHECK (backend IN ('yolov8n', 'yolov8s', 'onnx', 'coral')),
    model_path  TEXT        NOT NULL DEFAULT '' CHECK (length(model_path) <= 256),
    device      TEXT        NOT NULL DEFAULT 'cpu' CHECK (length(device) <= 16),
    enabled     BOOLEAN     NOT NULL DEFAULT FALSE,
    save_frames BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO detector_config DEFAULT VALUES;
