# domia-automation: instructions for Claude

Read `ROADMAP.md` (state and next steps), then the `[Unreleased]` section of `CHANGELOG.md`. Details live in `docs/` (setup of a development machine: `docs/dev-machine.md`; maintainer's runbook: `docs/releasing.md`; architecture decisions: `docs/adr/`). If a file named `CLAUDE.local.md` exists, read it too: it holds the owner's private notes and is never committed.

## What this is
A self-hosted home automation hub: a Rust workspace (Axum server, Leptos interface rendered on the server and in WebAssembly, sqlx with offline metadata), shipped as a Docker Compose stack (`deploy/`, driven by `tools/prod.sh` and `install.sh`). The internal product name is `dominet` (crates, binary, variables, default host name `dominet.local`); the repository is `domia-automation`. Licence AGPL-3.0-or-later.

## Rules that are never relaxed
- Never read or print a real `.env`, key, token, password or camera address. Never commit one.
- Never run `make prod-*`, `tools/prod.sh` or `install.sh` on a machine unless the owner asks for it; never touch a machine in production or its Zigbee dongle.
- **Never push, tag, create a release, publish an image or deploy the site without an explicit request each time.** An earlier approval does not cover the next one.
- Do not rewrite the history of `main`.
- One piece of work at a time: report, then wait for the owner's validation.

## Conventions
- Code, comments and identifiers in **English**, decimal point, no accents in code. User-facing texts are in `crates/dominet-web/locales/` (English and French).
- Documentation is bilingual: `x.md` is English, `x.fr.md` its French translation; change both, add new pages to `mkdocs.yml`; `make docs` (strict) must pass.
- `SPDX-License-Identifier: AGPL-3.0-or-later` at the top of every source file. `#![forbid(unsafe_code)]` (an exception needs its own crate and an ADR).
- A non-trivial architecture decision gets an ADR in `docs/adr/` (context, options, decision, consequences), in both languages.
- Commits: English, `scope: summary`, one logical change each, tests with the code they test. End with the `Co-Authored-By` line the environment asks for.
- Functional ambiguity: ask.

## The gate
- `make dev-up` starts the throw-away development stack (PostgreSQL, ReductStore, Mosquitto, MediaMTX); `make ci` is the gate (format, clippy `-D warnings`, tests, WebAssembly, deny, audit, sqlx metadata, installer checks); `make ci-full` adds the image and the browser end-to-end tests (before a release, or when the `Dockerfile` changes). No automatic CI service (ADR 0015).
- The `pre-push` hook (`make hooks`) reruns `make ci` on a clean copy and takes about 15 minutes: push with `GIT_SSH_COMMAND="ssh -o ServerAliveInterval=30 -o ServerAliveCountMax=60"` or GitHub closes the connection. Never `--no-verify`.
- Never chain `make ci; commit` without checking the exit code.
- The hook does not run the browser scripts: after a visible change, run `tools/e2e.sh <script>`.
- `make check-migrate` needs the ports free and the dev stack down then up. `tools/check-installer.sh` (in `make ci`) tests the installer with a fake `docker`.
- Never edit an applied SQL migration. After an automatic bulk replacement, run `cargo test --workspace --no-fail-fast` (strings compared by tests get broken silently).
- Before anything is made public, `tools/public-scan.sh` must have no unexplained finding.
