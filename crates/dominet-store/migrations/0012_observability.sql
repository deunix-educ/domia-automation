-- SPDX-License-Identifier: AGPL-3.0-or-later
-- R7 lot 2 (ADR 0052) : regles d'alerte de Prometheus et de Grafana, editees dans l'administration et ecrites en fichiers de
-- provisionnement (`rules.yml`, `rules.yaml`). Memes champs que `observability.AlertRule` et `GrafanaAlertRule` de Python.
-- (`alert_rule` existe deja : ce sont les regles d'alerte sur l'etat des appareils, R4.)

CREATE TABLE prometheus_rule (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name          TEXT        NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 128),
    rule_group    TEXT        NOT NULL DEFAULT 'dominet_custom' CHECK (length(rule_group) BETWEEN 1 AND 64),
    severity      TEXT        NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    expr          TEXT        NOT NULL CHECK (length(expr) BETWEEN 1 AND 4000),
    for_duration  TEXT        NOT NULL DEFAULT '5m' CHECK (length(for_duration) <= 16),
    summary       TEXT        NOT NULL CHECK (length(summary) BETWEEN 1 AND 256),
    description   TEXT        NOT NULL DEFAULT '' CHECK (length(description) <= 2000),
    eval_interval TEXT        NOT NULL DEFAULT '' CHECK (length(eval_interval) <= 16),
    active        BOOLEAN     NOT NULL DEFAULT TRUE,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE grafana_alert_rule (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    uid            TEXT        NOT NULL UNIQUE CHECK (uid ~ '^[a-z0-9_-]{1,64}$'),
    title          TEXT        NOT NULL CHECK (length(title) BETWEEN 1 AND 128),
    folder         TEXT        NOT NULL DEFAULT 'DomiNet Alerts' CHECK (length(folder) BETWEEN 1 AND 64),
    rule_group     TEXT        NOT NULL DEFAULT 'DomiNet' CHECK (length(rule_group) BETWEEN 1 AND 64),
    group_interval TEXT        NOT NULL DEFAULT '1m' CHECK (length(group_interval) <= 16),
    datasource_uid TEXT        NOT NULL DEFAULT 'prometheus' CHECK (length(datasource_uid) BETWEEN 1 AND 64),
    expr           TEXT        NOT NULL CHECK (length(expr) BETWEEN 1 AND 4000),
    query_window   INTEGER     NOT NULL DEFAULT 300 CHECK (query_window BETWEEN 1 AND 86400),
    for_duration   TEXT        NOT NULL DEFAULT '5m' CHECK (length(for_duration) <= 16),
    severity       TEXT        NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    summary        TEXT        NOT NULL CHECK (length(summary) BETWEEN 1 AND 256),
    description    TEXT        NOT NULL DEFAULT '' CHECK (length(description) <= 2000),
    no_data_state  TEXT        NOT NULL DEFAULT 'NoData' CHECK (no_data_state IN ('NoData', 'Alerting', 'OK')),
    exec_err_state TEXT        NOT NULL DEFAULT 'Error' CHECK (exec_err_state IN ('Error', 'Alerting', 'OK')),
    active         BOOLEAN     NOT NULL DEFAULT TRUE,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
