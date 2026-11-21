# 0024 — Browser end-to-end tests: stay on Marionette/Firefox

- Status: accepted
- Date: 2026-11-02

## Context
Playwright had been planned for the end-to-end tests. Since the first phase they run with Firefox driven by Marionette (a home-made, dependency-free client, `tests/e2e/marionette.py`), plugged into a throw-away database, the local binary and the Docker image (`tools/e2e.sh`). Node, needed by Playwright, is absent from the host (the production machine).

## Options
(a) **Stay on Marionette** — 11 scripts, 286 checks, nothing to install, already in `make ci-full`; the WebDriver pointer actions (`WebDriver:PerformActions`) suffice to test pointer drag and drop. (b) Install Node and Playwright — automatic waits and traces, but a second ecosystem on the production machine and a rewrite of the existing scripts.

## Decision
Option (a), until proven otherwise: reassess if Marionette cannot test live video.

Rules learned, to apply in every new script:
- **Wait for hydration to finish** (the "Real time: connected" indicator) before the first click: a button present and enabled before hydration has no effect.
- Use a **device number specific to the script**: the end-to-end server is shared and remembers each device's last state.
- Prepare data the interface cannot create through `E2E_DB_SHELL` (SQL on the throw-away database); prefer the interface when it can do it.
- A failure must be explainable: `tools/e2e.sh` shows the beginning of the server's panic or the end of its log, `KEEP_SERVER_LOG` keeps the log, `DOMINET_BIN`/`DOMINET_SITE` try another build profile.

## Consequences
- No Node on the host; end-to-end tests stay runnable on the production machine.
- No automatic waits: every wait is written (`wait_for`); an unstable test is a defect of the test **or** of the product, to elucidate (two Leptos rendering defects were found this way).
