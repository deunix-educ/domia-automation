-- SPDX-License-Identifier: AGPL-3.0-or-later
CREATE TABLE android_device (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    owner_id     BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         TEXT        NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    phone_number TEXT        NOT NULL DEFAULT '' CHECK (length(phone_number) <= 20),
    -- Secret used by the Android app to authenticate its WebSocket (regenerable).
    token        UUID        NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    is_online    BOOLEAN     NOT NULL DEFAULT FALSE,
    last_seen    TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX android_device_owner_idx ON android_device (owner_id, created_at DESC);

CREATE TABLE sms_message (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id    BIGINT      NOT NULL REFERENCES android_device (id) ON DELETE CASCADE,
    direction    TEXT        NOT NULL CHECK (direction IN ('incoming', 'outgoing')),
    phone_number TEXT        NOT NULL CHECK (length(phone_number) BETWEEN 1 AND 30),
    contact_name TEXT        NOT NULL DEFAULT '' CHECK (length(contact_name) <= 100),
    body         TEXT        NOT NULL,
    is_read      BOOLEAN     NOT NULL DEFAULT FALSE,
    sent_at      TIMESTAMPTZ NOT NULL,
    received_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sms_message_device_idx ON sms_message (device_id, sent_at DESC);
CREATE INDEX sms_message_unread_idx ON sms_message (device_id) WHERE NOT is_read;

CREATE TABLE call_log (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id    BIGINT      NOT NULL REFERENCES android_device (id) ON DELETE CASCADE,
    direction    TEXT        NOT NULL CHECK (direction IN ('incoming', 'outgoing', 'missed')),
    phone_number TEXT        NOT NULL CHECK (length(phone_number) BETWEEN 1 AND 30),
    contact_name TEXT        NOT NULL DEFAULT '' CHECK (length(contact_name) <= 100),
    started_at   TIMESTAMPTZ NOT NULL,
    ended_at     TIMESTAMPTZ,
    duration     INTEGER     NOT NULL DEFAULT 0 CHECK (duration >= 0),   -- seconds
    received_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX call_log_device_idx ON call_log (device_id, started_at DESC);
