# 0011 — Encryption of sensitive values at rest

- Status: accepted
- Date: 2026-11-01

## Context
Application settings contain secrets (a Free Mobile API key today; camera passwords, tokens and SMTP credentials later). PostgreSQL is backed up (`pg_dump`) and can be copied: a clear-text value in a dump or backup is a leak.

## Options
1. Clear-text values, the database being "private" — simple, but any dump, backup or read-only access exposes the secrets.
2. Disk or PostgreSQL encryption (TDE) — does not protect against a logical dump or SQL access.
3. **Application-level encryption of sensitive values** with a master key kept outside the database — chosen.

## Decision
- **AES-256-GCM** (crate `aes-gcm` 0.11), a 32-byte master key in base64 in `DOMINET_MASTER_KEY` (the `.env` file, never in the database or in git). `dominet gen-master-key` generates one.
- **Stored format**: `v1:` + base64( 96-bit nonce ‖ ciphertext ‖ tag ). The version prefix allows changing the algorithm later.
- **Associated data = the setting name**: a ciphertext copied into another setting's row does not decrypt (tested).
- **Random nonce** at every encryption: two ciphertexts of the same text differ.
- **Which settings**: those whose key contains `KEY`, `PASSWORD`, `TOKEN` or `SECRET`, marked `is_secret`; the domain refuses to save a sensitive key not marked secret.
- **Empty value = stored empty**, unencrypted (nothing to protect); this allows clearing a secret without the master key.
- **No read-back**: the API returns `********` for a set secret; a secret is write-only. Values are never logged (only the setting name is).
- **In-memory cache** of decrypted values for internal use (sending SMS, etc.); it disappears with the process.
- **Write refused without a master key** (explicit 503) rather than saved in clear. `seed` checks everything before writing: an error writes nothing.
- The key is **validated at start-up** (`Config`): a key of the wrong length stops the server with a clear message.

## Consequences
- **Losing the master key makes the secrets unrecoverable** (they must be entered again). It must be backed up separately from the database. Changing the key makes existing secrets unreadable; they are flagged (`readable = false`, 503 on use) and the other settings keep working.
- No automatic rotation: to change the key, read the secrets with the old one then rewrite them with the new one (a tool to add if the need appears).
- Encryption protects the database **at rest**; an attacker who controls the server process (and so the key in memory) is not covered.
- The same mechanism serves camera passwords.
