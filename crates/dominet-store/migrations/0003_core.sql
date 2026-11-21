-- SPDX-License-Identifier: AGPL-3.0-or-later
CREATE TABLE app_setting (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    key         TEXT        NOT NULL UNIQUE CHECK (length(key) BETWEEN 1 AND 64),
    -- Plain text, or AES-256-GCM ciphertext (base64) when is_secret is true (ADR 0011).
    value       TEXT        NOT NULL,
    value_type  TEXT        NOT NULL DEFAULT 'str' CHECK (value_type IN ('str', 'int', 'float', 'bool')),
    category    TEXT        NOT NULL DEFAULT 'Notifications',
    description TEXT        NOT NULL DEFAULT '',
    is_secret   BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX app_setting_category_idx ON app_setting (category, key);
CREATE TRIGGER app_setting_set_updated_at BEFORE UPDATE ON app_setting
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE navbar_item (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    icon_name  TEXT     NOT NULL CHECK (length(icon_name) BETWEEN 1 AND 64),   -- Lucide icon name
    label      TEXT     NOT NULL CHECK (length(label) BETWEEN 1 AND 128),
    -- Application path ("/surveillance/") or external URL ("https://...").
    url        TEXT     NOT NULL CHECK (length(url) BETWEEN 1 AND 255),
    url_target TEXT     NOT NULL DEFAULT '_self',
    admin_only BOOLEAN  NOT NULL DEFAULT FALSE,
    position   TEXT     NOT NULL DEFAULT 'top' CHECK (position IN ('top', 'bottom')),
    sort_order SMALLINT NOT NULL DEFAULT 0 CHECK (sort_order >= 0)
);
CREATE INDEX navbar_item_order_idx ON navbar_item (position, sort_order, label);

CREATE TABLE location (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name        TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    description TEXT    NOT NULL DEFAULT '' CHECK (length(description) <= 128),
    active      BOOLEAN NOT NULL DEFAULT TRUE
);
CREATE INDEX location_name_idx ON location (name);
