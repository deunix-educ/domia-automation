-- SPDX-License-Identifier: AGPL-3.0-or-later
-- R11 lot M1 (ADR 0059) : jetons d'acces au serveur MCP et journal d'audit.
-- Un jeton n'est JAMAIS stocke en clair : seul son SHA-256 l'est (la valeur est aleatoire, 256 bits).
CREATE TABLE mcp_token (
    id           BIGSERIAL PRIMARY KEY,
    name         TEXT        NOT NULL,
    token_hash   BYTEA       NOT NULL UNIQUE,
    prefix       TEXT        NOT NULL,
    scopes       TEXT[]      NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by   BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    -- Pas de jeton sans date de fin.
    expires_at   TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ,
    CHECK (octet_length(token_hash) = 32),
    CHECK (cardinality(scopes) > 0)
);

CREATE TABLE mcp_audit (
    id         BIGSERIAL PRIMARY KEY,
    at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    kind       TEXT        NOT NULL,
    actor      TEXT        NOT NULL,
    actor_kind TEXT        NOT NULL,
    token_id   BIGINT      REFERENCES mcp_token (id) ON DELETE SET NULL,
    tool       TEXT        NOT NULL DEFAULT '',
    params     JSONB       NOT NULL DEFAULT '{}',
    outcome    TEXT        NOT NULL,
    detail     TEXT        NOT NULL DEFAULT ''
);
CREATE INDEX mcp_audit_at ON mcp_audit (at DESC);
CREATE INDEX mcp_audit_kind_at ON mcp_audit (kind, at DESC);
