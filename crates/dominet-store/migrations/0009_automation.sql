-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Scenes, automations, alertes et bilan d'energie (R4, ADR 0031).
--
-- Les declencheurs et actions d'une automation gardent la forme JSON de la version Python (`trigger_type` + `trigger_config`,
-- `action_type` + `action_config`) : les scenarios de la production se chargent tels quels. Ils sont valides par le domaine.
-- `python_plugin` devient `script` (Rhai).

CREATE TABLE scene (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- Unique : l'API des scripts active une scene par son nom (`PluginContext.scene(name)` plantait sur un doublon en Python).
    name        TEXT   NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 64),
    icon        TEXT   NOT NULL DEFAULT '' CHECK (length(icon) <= 32),
    description TEXT   NOT NULL DEFAULT '' CHECK (length(description) <= 256),
    location_id BIGINT REFERENCES location (id) ON DELETE SET NULL,
    active      BOOLEAN NOT NULL DEFAULT TRUE
);

-- Commandes d'une scene, publiees dans l'ordre de `position`.
CREATE TABLE scene_action (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    scene_id  BIGINT   NOT NULL REFERENCES scene (id) ON DELETE CASCADE,
    device_id BIGINT   NOT NULL REFERENCES device (id) ON DELETE CASCADE,
    payload   JSONB    NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    position  SMALLINT NOT NULL DEFAULT 0 CHECK (position >= 0)
);
CREATE INDEX scene_action_scene_idx ON scene_action (scene_id, position, id);
CREATE INDEX scene_action_device_idx ON scene_action (device_id);

-- Le widget « scene » (R2) pointe maintenant sur une vraie scene : references orphelines a NULL, puis cle etrangere.
UPDATE widget SET scene_id = NULL WHERE scene_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM scene s WHERE s.id = widget.scene_id);
ALTER TABLE widget ADD CONSTRAINT widget_scene_fk FOREIGN KEY (scene_id) REFERENCES scene (id) ON DELETE SET NULL;

CREATE TABLE automation (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name           TEXT        NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    enabled        BOOLEAN     NOT NULL DEFAULT TRUE,
    description    TEXT        NOT NULL DEFAULT '' CHECK (length(description) <= 256),
    trigger_type   TEXT        NOT NULL CHECK (trigger_type IN ('mqtt_message', 'schedule', 'solar', 'device_state')),
    trigger_config JSONB       NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(trigger_config) = 'object'),
    -- Condition facultative {key, value} ; `{}` = aucune.
    condition      JSONB       NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(condition) = 'object'),
    action_type    TEXT        NOT NULL CHECK (action_type IN ('mqtt_publish', 'script', 'notification', 'scene')),
    action_config  JSONB       NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(action_config) = 'object'),
    -- Script Rhai de l'action `script` (execute a partir du lot 5).
    script         TEXT        NOT NULL DEFAULT '' CHECK (length(script) <= 20000),
    last_triggered TIMESTAMPTZ,
    trigger_count  INTEGER     NOT NULL DEFAULT 0 CHECK (trigger_count >= 0),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX automation_name_idx ON automation (name);
CREATE INDEX automation_enabled_trigger_idx ON automation (trigger_type) WHERE enabled;
CREATE TRIGGER automation_set_updated_at BEFORE UPDATE ON automation
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Journal d'execution des automations (une ligne par declenchement), purge avec l'historique.
CREATE TABLE automation_run (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    automation_id BIGINT      NOT NULL REFERENCES automation (id) ON DELETE CASCADE,
    at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Ce qui a declenche : topic du message, `schedule`, `solar`, `manual` (essai).
    trigger       TEXT        NOT NULL DEFAULT '' CHECK (length(trigger) <= 256),
    status        TEXT        NOT NULL CHECK (status IN ('ok', 'error', 'skipped', 'limited')),
    duration_ms   INTEGER     NOT NULL DEFAULT 0 CHECK (duration_ms >= 0),
    message       TEXT        NOT NULL DEFAULT '' CHECK (length(message) <= 1000)
);
CREATE INDEX automation_run_recent_idx ON automation_run (automation_id, at DESC);
CREATE INDEX automation_run_at_idx ON automation_run (at);

-- Un travail du planificateur peut appartenir a une automation (supprime avec elle) : voir `schedule_job` (migration 0006).
ALTER TABLE schedule_job ADD COLUMN automation_id BIGINT REFERENCES automation (id) ON DELETE CASCADE;
CREATE UNIQUE INDEX schedule_job_automation_idx ON schedule_job (automation_id) WHERE automation_id IS NOT NULL;
-- Les neuf evenements solaires (la migration 0006 n'en admettait que deux).
ALTER TABLE schedule_job DROP CONSTRAINT schedule_job_solar_event_check;
ALTER TABLE schedule_job ADD CONSTRAINT schedule_job_solar_event_check CHECK (solar_event IN (
    'sunrise', 'sunset', 'solar_noon', 'dawn_civil', 'dusk_civil', 'dawn_nautical', 'dusk_nautical',
    'dawn_astronomical', 'dusk_astronomical'));

CREATE TABLE alert_rule (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name             TEXT             NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    device_id        BIGINT           NOT NULL REFERENCES device (id) ON DELETE CASCADE,
    key              TEXT             NOT NULL CHECK (length(key) BETWEEN 1 AND 64),
    operator         TEXT             NOT NULL DEFAULT 'gt' CHECK (operator IN ('gt', 'gte', 'lt', 'lte', 'eq', 'neq', 'contains')),
    threshold_num    DOUBLE PRECISION,
    threshold_str    TEXT             NOT NULL DEFAULT '' CHECK (length(threshold_str) <= 128),
    severity         TEXT             NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    cooldown_minutes SMALLINT         NOT NULL DEFAULT 10 CHECK (cooldown_minutes >= 0),
    notify_email     BOOLEAN          NOT NULL DEFAULT TRUE,
    notify_sms       BOOLEAN          NOT NULL DEFAULT FALSE,
    notify_android   BOOLEAN          NOT NULL DEFAULT TRUE,
    enabled          BOOLEAN          NOT NULL DEFAULT TRUE,
    description      TEXT             NOT NULL DEFAULT '' CHECK (length(description) <= 256)
);
CREATE INDEX alert_rule_device_idx ON alert_rule (device_id) WHERE enabled;

CREATE TABLE alert (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    rule_id         BIGINT      REFERENCES alert_rule (id) ON DELETE SET NULL,
    device_id       BIGINT      NOT NULL REFERENCES device (id) ON DELETE CASCADE,
    status          TEXT        NOT NULL DEFAULT 'firing' CHECK (status IN ('firing', 'resolved', 'acknowledged')),
    severity        TEXT        NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    value_triggered TEXT        NOT NULL CHECK (length(value_triggered) <= 128),
    fired_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    acknowledged_at TIMESTAMPTZ,
    acknowledged_by BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    notified        BOOLEAN     NOT NULL DEFAULT FALSE
);
CREATE INDEX alert_status_fired_idx ON alert (status, fired_at DESC);
CREATE INDEX alert_rule_device_status_idx ON alert (rule_id, device_id, status);

-- Bilan energetique journalier (Linky + SolarMax + PAC), un par jour local.
CREATE TABLE energy_daily_log (
    day         DATE    PRIMARY KEY,
    linky_hp_wh INTEGER NOT NULL DEFAULT 0,
    linky_hc_wh INTEGER NOT NULL DEFAULT 0,
    solar_wh    INTEGER NOT NULL DEFAULT 0,
    pac_wh      INTEGER NOT NULL DEFAULT 0
);
