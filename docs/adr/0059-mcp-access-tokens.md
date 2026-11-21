# ADR 0059 — MCP access tokens: hashed, scoped, revocable, with an audit trail

Status: accepted: personal tokens created in the administration.

## Context
The interface authenticates with a session cookie, unusable by an MCP client. An assistant that changes the configuration must identify itself, have limited rights and leave a trace. The service is reachable from the LAN and the VPN only.

## Options
1. **A single token in an environment variable**: simple, but no scope, no individual revocation, no per-token trace. Kept **only** as a provisional token at the very beginning.
2. **OAuth 2.1** (planned by the MCP specification for public servers): heavy, useless on a private network. Discarded.
3. **Personal tokens created in the administration** (chosen).

## Decision
- **Creation** by an administrator, in the administration page; the value is **shown only once**. Format: 32 random bytes, prefix `dnm_` (recognisable by a secret scanner), URL-safe encoding.
- **Hashed storage** (SHA-256 of the value: the token is random with high entropy, a slow hash adds nothing) with: name, scopes, creation date, mandatory expiry date (90 days by default, 365 at most), last use, state (active / revoked). Never the clear value, neither in the database, nor in the logs, nor in backups.
- **Scopes**, cumulative: `read`, `control`, `configuration`, `administration`. A tool declares its scope; the call is refused without it. Advice: a `read` token for everyday diagnostics.
- **Immediate revocation** (a revoked or expired token is refused at the next request).
- **Rate limiting** per token.
- **Audit trail**: every call that **changes** something is recorded (token, tool, parameters without secrets, result, instant), visible in the administration. The built-in assistant records its actions there with the `agent` mark and the session's account.
- **Actor**: the common service receives an actor (`Token{scopes}` or `Session{account}`), not just an account, so that the built-in assistant (no token) and the interface share the same code.
- **Out of scope, permanently**: creating tokens, accounts and passwords, MQTT accounts, messaging, backups, any access to the file system or the shell.

## Consequences
- A token table and an audit table, two administration pages, a `Bearer` extractor.
- A lost token is revoked and recreated; there is no way to read it back.
- The audit log grows with use: purge after a retention period.

## Implementation notes
- **Format**: `dnm_` + 43 characters (32 random bytes in unpadded URL-safe base64). The visible beginning is `dnm_` + 8 characters (48 bits out of 256).
- **`mcp_token` table**: SHA-256 fingerprint (32 bytes, length and uniqueness constraints), scopes as `TEXT[]` (at least one), **mandatory** `expires_at`, `last_used_at` (written at most once a minute), `revoked_at`. No column can hold the value; a test checks it on the raw row content.
- **Authentication** on every request, from the database: a revoked or expired token is refused at the next call, with no cache.
- **Scopes enforced in a single place** (`call_tool` and `list_tools` of `dominet-mcp`): a tool absent from the `SCOPES` table is refused to everyone, and a test requires every tool to appear there. A refusal returns an explicit error result (the assistant can explain it) and writes a `denied` row in the audit.
- **Rate limit**: sliding one-minute window per token, in memory (`MCP_RATE_LIMIT_PER_MIN`, 120); beyond it, 429 with `Retry-After`. A restart resets the counters. Refused calls are not counted.
- **Audit** (`mcp_audit`): `token_created`, `token_revoked`, `denied` and `tool_call`. Parameters go through `redact` (keys `password`, `secret`, `token`, `key`, `credential`, `authorization`… masked at any depth, long texts cut at 500 characters). Daily purge beyond `MCP_AUDIT_RETENTION_DAYS` (365).
- **For tools that change something, the audit is written before execution** (a `pending` row, closed as `ok` or `error`) **and the call is refused if the write fails**; a test drops the audit table and checks that nothing is published. For token creation and revocation (done by a signed-in administrator) the audit write is best effort.
- **Token owner**: `Actor::Token` carries the account that created it. Acknowledging an alert is attributed to it; a token whose creator was deleted can no longer acknowledge.
- **Actor**: `Actor::Token{id, name, scopes}` or `Actor::Session{account, role}` in `dominet-domain`; an administrator session holds all four scopes, any other account `read` and `control`.
