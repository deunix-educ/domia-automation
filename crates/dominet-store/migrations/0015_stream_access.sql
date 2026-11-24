-- SPDX-License-Identifier: AGPL-3.0-or-later
-- ADR 0064 lot 1: secrets that let an account watch the cameras (viewing tokens for the browser, personal stream credentials
-- for external players). A secret is NEVER stored in clear: only its SHA-256 (the value is random, 256 bits).
CREATE TABLE stream_access (
    id           BIGSERIAL PRIMARY KEY,
    user_id      BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL CHECK (kind IN ('viewing', 'credential')),
    -- The device a credential is for; empty for a viewing token.
    name         TEXT        NOT NULL DEFAULT '' CHECK (length(name) <= 64),
    secret_hash  BYTEA       NOT NULL UNIQUE,
    prefix       TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- No secret without an end date.
    expires_at   TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ,
    CHECK (octet_length(secret_hash) = 32),
    CHECK (kind <> 'credential' OR length(name) > 0)
);
CREATE INDEX stream_access_user ON stream_access (user_id, created_at DESC);
CREATE INDEX stream_access_expires ON stream_access (expires_at);
