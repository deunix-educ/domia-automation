-- SPDX-License-Identifier: AGPL-3.0-or-later
CREATE TABLE users (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username            TEXT        NOT NULL CHECK (length(username) BETWEEN 1 AND 150),
    email               TEXT        NOT NULL DEFAULT '',
    first_name          TEXT        NOT NULL DEFAULT '',
    last_name           TEXT        NOT NULL DEFAULT '',
    -- Mobile number used for SMS alerts (e.g. +33612345678).
    mobile              TEXT        NOT NULL DEFAULT '' CHECK (length(mobile) <= 20),
    -- argon2id PHC string; never a clear-text password.
    password_hash       TEXT        NOT NULL,
    role                TEXT        NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user')),
    is_active           BOOLEAN     NOT NULL DEFAULT TRUE,
    last_login_at       TIMESTAMPTZ,
    password_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Case-insensitive uniqueness ("Bob" and "bob" are the same account).
CREATE UNIQUE INDEX users_username_lower_key ON users (lower(username));
CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Server-side sessions (tower-sessions store, see ADR 0004).
CREATE TABLE session (
    id          TEXT        PRIMARY KEY,
    data        JSONB       NOT NULL DEFAULT '{}'::jsonb,
    expiry_date TIMESTAMPTZ NOT NULL,
    -- Set at login so every session of an account can be revoked at once.
    user_id     BIGINT      REFERENCES users (id) ON DELETE CASCADE
);
CREATE INDEX session_expiry_date_idx ON session (expiry_date);
CREATE INDEX session_user_id_idx ON session (user_id);

-- Login attempts, used by the progressive lockout (per account and per IP).
CREATE TABLE login_attempt (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username     TEXT        NOT NULL,   -- lowercased, may not match any account
    ip           TEXT        NOT NULL,
    succeeded    BOOLEAN     NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX login_attempt_username_idx ON login_attempt (username, attempted_at DESC);
CREATE INDEX login_attempt_ip_idx ON login_attempt (ip, attempted_at DESC);
