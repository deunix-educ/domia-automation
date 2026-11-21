# Contributing

Thank you for helping. Small, focused changes are the easiest to review.

## Before you start

- Open an issue first for anything larger than a fix, so we agree on the direction. Architecture decisions are written as ADRs in `docs/adr/`; a non-trivial change adds one.
- The project is licensed **AGPL-3.0-or-later**. By contributing you agree that your contribution is distributed under that licence. Every source file starts with `SPDX-License-Identifier: AGPL-3.0-or-later` (`//` or `#`).

## Build and check

```bash
make dev-up            # throw-away PostgreSQL, ReductStore, Mosquitto, MediaMTX for the tests
make ci                # format, clippy -D warnings, tests, WebAssembly, cargo deny, cargo audit, sqlx metadata, installer checks
make ci-full           # + container image and browser end-to-end tests (before a release, or when the Dockerfile changes)
make docs              # MkDocs, strict: no broken link
```

There is **no automatic CI service** (decision recorded in ADR 0015): `make ci` is the gate and the `pre-push` hook (`make hooks`) reruns it on a clean copy. Please run it before you open a pull request and say so in the description.

## Conventions

- **Code, comments and identifiers in English**, decimal point, no accents in code. User-facing texts live in the translation files (`crates/dominet-web/locales/`), English and French.
- **Documentation is bilingual**: every page `x.md` (English) has an `x.fr.md`; change both. New pages go in the navigation of `mkdocs.yml`.
- `#![forbid(unsafe_code)]` (workspace lints); an exception needs its own crate and an ADR.
- Commits: English, `scope: summary`, one logical change each. Tests with the code they test.
- Never commit a real `.env`, key, password, camera address or image of a person. `tools/public-scan.sh` looks for the usual traces.

## Reporting bugs

Use the issue template. Run `make prod-doctor` and `make prod-report` and attach the output (read it first: it holds no password, but it may hold your host name and addresses).

## Security problems

Do not file them as issues: see [SECURITY.md](SECURITY.md).
