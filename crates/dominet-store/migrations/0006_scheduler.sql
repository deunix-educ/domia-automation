-- SPDX-License-Identifier: AGPL-3.0-or-later
-- In-process scheduler jobs: interval, cron and solar schedules.
CREATE TABLE schedule_job (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name            TEXT             NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 200),
    -- Job kind handled by the scheduler (e.g. compute_daily_energy, run_backup, run_automation).
    task            TEXT             NOT NULL CHECK (length(task) BETWEEN 1 AND 100),
    args            JSONB            NOT NULL DEFAULT '{}'::jsonb,
    kind            TEXT             NOT NULL CHECK (kind IN ('interval', 'cron', 'solar')),
    interval_every  INTEGER,
    interval_period TEXT             CHECK (interval_period IN ('minutes', 'hours', 'days')),
    cron_expr       TEXT,
    timezone        TEXT             NOT NULL DEFAULT 'Europe/Paris',
    solar_event     TEXT             CHECK (solar_event IN ('sunrise', 'sunset')),
    latitude        DOUBLE PRECISION CHECK (latitude BETWEEN -90 AND 90),
    longitude       DOUBLE PRECISION CHECK (longitude BETWEEN -180 AND 180),
    enabled         BOOLEAN          NOT NULL DEFAULT TRUE,
    last_run_at     TIMESTAMPTZ,
    next_run_at     TIMESTAMPTZ,
    last_status     TEXT             NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ      NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ      NOT NULL DEFAULT now(),
    -- Each kind carries exactly the fields it needs.
    CONSTRAINT schedule_job_kind_fields CHECK (
        (kind = 'interval' AND interval_every > 0 AND interval_period IS NOT NULL)
        OR (kind = 'cron' AND cron_expr IS NOT NULL AND length(cron_expr) > 0)
        OR (kind = 'solar' AND solar_event IS NOT NULL AND latitude IS NOT NULL AND longitude IS NOT NULL)
    )
);
CREATE INDEX schedule_job_due_idx ON schedule_job (next_run_at) WHERE enabled;
CREATE TRIGGER schedule_job_set_updated_at BEFORE UPDATE ON schedule_job
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
