# Pitfalls and lessons

Every trap met while building the application, by area. Each line says what you see, why it happens, what fixes it and, when there is one, what now guards against it. Read this before rebuilding or porting any part: none of these is obvious in advance and most were found by a test, a build of the image or the real machine, not by reading code.

## 1. Build and toolchain

| Symptom | Cause | Remedy / guard |
|---|---|---|
| Buttons inert, no message in the browser | The server and the WASM are not compiled alike. `cargo leptos` adds `--cfg erase_components` and `LEPTOS_OUTPUT_NAME`; a plain `cargo build` or `cargo test` does not, so the server HTML no longer matches the WASM and hydration fails | Both settings live in `.cargo/config.toml` for every build (do not remove them). `ui.rs` checks the WASM name and the hydration markers; the E2E run on the image is the reference test |
| E2E failures that make no sense | Stale WASM or stale image: the interface or the image was not rebuilt | Always `cargo leptos build --release`, then `cargo build -p dominet` before the E2E; `make e2e-image` does not rebuild, so run `make image-smoke e2e-image` |
| `make ci` is green, the Docker image does not build | The Dockerfile copies only `crates/` and `.sqlx/`; an `include_str!` pointing outside `crates/` compiles locally | Nothing may be included from outside `crates/`; run `make ci-full` before a milestone or when the Dockerfile or a native dependency changes |
| Image cannot link ONNX Runtime | The prebuilt C++ library needs `g++` at build time, `libstdc++6` at run time and glibc 2.38 or newer | Rebuild the image as soon as a native dependency is added |
| Server panics at start when `REDUCTSTORE_URL` is set, only in the image | The image has no root certificates; the rustls HTTP client panics when built | The image installs `ca-certificates`; client construction no longer panics |
| Image build fails randomly | `wasm-bindgen` and `wasm-opt` are downloaded from a CDN | The Dockerfile retries four times; otherwise run it again |
| Risk of an unusable binary on the target machine | Building with `-C target-cpu=native` ties the binary to the build machine's CPU (the target is a Skylake without AVX-512) | Never publish with `target-cpu=native`; baseline is `x86-64-v2` (ADR 0001) |
| `sqlx` refuses a query | `sqlx` 0.9 requires `AssertSqlSafe` for any dynamically built SQL | Wrap dynamic SQL deliberately, never concatenate user input |
| Offline compilation fails after adding a query | The `.sqlx/` metadata is out of date | Any new `sqlx::query!` is followed by `make sqlx-prepare`; `make sqlx-check` guards it |
| Clippy flags product names in doc comments | `clippy::doc_markdown` | Declare proper nouns in `clippy.toml` |
| Dependencies drift into advisories | A transitive crate gets a RUSTSEC entry (a mail crate did) | `cargo update -p <crate>`; `make ci` runs `cargo deny` and `cargo audit` |

## 2. Web interface (Leptos, server-side rendering)

- **Page frozen after a cold start** (Leptos 0.8.23): concurrent renders may never finish, in 1 to 5 % of cold starts. The "reactive value disposed" panics in the log are a consequence, not the cause. Guard: `web/render_guard.rs` abandons and retries a `GET` that gets no answer in 5 s (3 tries, then 503); reproducer `tools/stress-cold-render.sh` (ADR 0021).
- Read a resource under `Transition`, otherwise server rendering does not wait for it. Render routes with `SsrMode::Async`.
- No inline `style=` or `on…=` (content security policy). `prop:value` is not rendered on the server: use `value=` for the initial state and `disabled=` instead of `prop:disabled`.
- No `StoredValue::new_local` in a component rendered on the server (a `SendWrapper` panic on release freezes the page).
- No resource whose source depends on another resource (loads arrive out of order at hydration and the table is stale).
- No curly braces in a `.ftl` file (panic while loading translations).
- Read form fields through `NodeRef` and the input value, not signals: browser autofill fills fields without firing `input`.
- Several Leptos apps rendered in parallel inside one test process disturb each other: `ui.rs` runs them one at a time under a lock.
- Links in the navigation bar must be loaded by the browser, not by the in-app router, when the target is served by the server (`/docs/`, `/grafana/`): the router answers "page not found".
- A required field in a form that is not needed for every case blocks creation (a Grafana widget could not be created because "device / scene / camera" were all required).
- A tab left open during an update keeps the old front end and can show a badge without its list. `/pkg/` is served `no-cache` with an ETag; a hard reload fixes it.
- The development WASM is 48 MB (buttons inert for 10 to 30 s); use a release build for demos.

## 3. Authentication and sessions

- `flush()` alone does not renew the session identifier: use `flush` then `cycle_id`, or the session is fixed.
- A stolen session could guess the current password without limit: apply the same lockout as the login.
- Two administrators demoting each other at the same time left no administrator: an advisory lock serialises it (race test).
- MCP tokens: hashed, scoped, with a mandatory end date, immediate revocation, rate limit; scopes are enforced in a single place.

## 4. Database and tests

- Creating throw-away databases (`#[sqlx::test]`) forces a checkpoint; at 20 test threads `PoolTimedOut` appears about one run in three. The test run is bounded to 8 threads (`TEST_THREADS`). The same `PoolTimedOut` appears on every database test when the development database is simply not running.
- Race conditions found only by deliberate race tests: a default dashboard read before its lock, two concurrent widget insertions blocking on an exclusion constraint (take the row lock first).
- `pg_dump` 17 against a PostgreSQL 16 server writes `SET transaction_timeout`, unknown to the server: drop that line when restoring without the tool.
- A long task at start-up (resume jobs) must run in the background; waiting for the database before starting the HTTP server made the image smoke test fail when the database is unreachable.
- A test must not assume an empty broker or an already loaded engine: engines reload their list in the background, so tests repeat the action until the first reaction.

## 5. MQTT and Zigbee

| Symptom | Cause | Remedy |
|---|---|---|
| A remote-control press is lost | Ingestion kept only the last state of a device received within one second; a remote sends `action: 1_single` then `action: ""` milliseconds apart | Automations also see the grouped states (ADR 0032) |
| Device missing after creating an MQTT source | A **retained** message arrived before "automatic device" was ticked | Tick it at creation; a retained device otherwise appears only on re-subscription |
| Throughput test unstable under load | Mosquitto **drops** messages beyond 1 000 queued per client | `max_queued_messages 100000` in development and production |
| Anonymous access leaks to the secured listener | Without `per_listener_settings`, `allow_anonymous true` applies everywhere | Set `per_listener_settings true`; one password-protected listener, per-client accounts (ADR 0057) |
| Pairing button does nothing | Zigbee2MQTT 2.x rejects `{"value": true}` ("Invalid payload") | Send `{"time": 254}` |
| A device already paired elsewhere will not pair | It remembers its old network | Reset it first |
| Renaming a device breaks its commands | A device's `friendly_name` is also its MQTT topic | Rename from the administration pages, knowing the topic changes |
| Blind stops mid-movement impossible | The relay has no `moving` key and no `STOP` state: its state derives from the position and it publishes about one message a second while moving | Use the age of the last message (`age_secs` in scripts). Never write a script from a plugin's name |
| A refusal message stays after the broker returns | The notice was cleared only by user action | It clears as soon as the device publishes a state |
| Throw-away tests see old Zigbee messages | The development broker keeps retained messages | Per-test topic prefix and a redirected `Z2M_BASE_TOPIC` |

## 6. Video and detection

- **Software decoding** in the container used 85 to 90 % of a core: the container had no `/dev/dri`, no `render` group and no driver. Fix: VAAPI (`make prod-video`), measured at 36 %.
- The VAAPI fallback was triggered too eagerly by an offline camera. Only the real machine revealed it.
- ReductStore 1.21 and later refuses an empty `RS_API_TOKEN`; its image has neither `wget` nor `curl` (health check through `bash` and `/dev/tcp`).
- MediaMTX: the configuration schema changes between versions; revalidate on every upgrade. The ICE port (8189, UDP and TCP) is fixed, so only one stack at a time.
- The password of a camera appears in clear in an ONVIF `GetStreamUri` answer: mask it everywhere.
- Only one TCP client at a time on the solar inverter: a diagnostic read fails while the publisher role is running.
- Test fixtures for ONVIF are rebuilt from captures; confront them with real cameras with the ignored `real_onvif` test.
- An endurance script that waits for indexed segments but not archived ones races.

## 7. Domain logic

- `float("1_000")` is valid in Python and refused by Rust: the alert rule parser had to accept it (found by comparing with the reference implementation).
- Times shown in UTC (`…Z`) in the automation list: the computation is right (solar events are absolute instants, schedules go through `TZ`, default `Europe/Paris`, with ambiguous and missing hours handled) but the display should be local.
- Energy balance: truncation rather than rounding (`int(1.005 × 1000)`), daylight-saving nights, local-day edges, counter reset, days without data. These are covered by golden fixtures in `tests/fixtures/energy`.
- Never copy a coordinate from an example file: weather and solar times were wrong for a whole installation because sample coordinates were kept.

## 8. Observability (Prometheus, Grafana)

- Grafana needs an SMTP sender address with a dot and a non-empty recipient, otherwise it restarts in a loop. A volume mounted `:ro` prevents nested mounts.
- Grafana alert rules with `NoData` set to "no data" fire forever: "no result" means "all is well" for these rules. Default to `OK`; fix existing installations by hand.
- 288 failed e-mails in 12 hours: a stale SMTP password (services not recreated after editing `.env`) and a recipient written `=…`.
- Compose interpolates variables of inactive profiles too: `prod-guard` checks them.
- Prometheus must load every rule group; a count taken from the rules file is the right check (`tools/check-observability.sh`).
- Platform alerts in `/alerts` read Prometheus `GET /api/v1/alerts` (state `firing`) and Grafana's alert manager API with basic authentication; the hub is in-process, so the poller lives in the web role (ADR 0061).
- The energy dashboard finds devices by state keys (`pac`, `qdy`, `PAPP`, `power_ab`); those keys must be **tracked** in the device history, which `dominet seed-energy` now does.

## 9. Production and operations

| Symptom | Cause | Remedy |
|---|---|---|
| `prod-deploy` stops before any build | Compose 5.6 delegates to `buildx bake`, which rejects `network: host` in `build` | Remove the line |
| Firefox refuses the certificate, Brave accepts it | Firefox has its own certificate store | Import the CA by hand (or `security.enterprise_roots.enabled`); `libnss3-tools` for `mkcert -install` |
| E-mails sent to `=address` | Double `=` in `.env` (`NAME==value`) | `prod-guard` reports it |
| Changed `.env`, nothing changes | The service was not recreated | Recreate the services after any `.env` change |
| `make prod-logs` blocks with `grep` | It follows continuously | `N=100` for the last lines without following |
| Disk full on the production machine | A 370 GB `target/` and a 73 GB build cache left on it | Never build on the production machine |
| Push fails for the 7-minute hook | GitHub drops the connection during the hook | `ssh -o ServerAliveInterval=30` |
| The `pre-push` hook fails on MQTT tests | It uses the development stack, which was stopped | `make dev-up` first |
| Backup summary showed no PostgreSQL backups | It kept the 4 latest lines, all kinds mixed | Two per kind |
| Login button inert in one browser | Interface did not start in the tab; cause never identified | Clear the cache or disable the browser's shields |

## 10. End-to-end tests

- A click before hydration "succeeds" and does nothing: wait for the real-time indicator or repeat the click until the form opens (met four times).
- The simulated SMTP server is shared by the scripts: filter on your own subject. The E2E server remembers the last device state: give each script its own device number.
- Two scripts run together can interfere (`alerts` with `platform_alerts`: the fake Prometheus is counted in the badge). Run them separately.
- Two unexplained flaky steps, never reproduced: the memory bar width in `system` and the `publish solarmax` diagnostic. Note, do not hide.
- Run `make ci` often: a break in the interface tests (truncated translations) was seen only at the end of a lot.

## 11. MCP and the local assistant

- A script reaches the application through the back door: `save_automation` and `save_mqtt_source` accept script code, so a `configuration` token could write scripts. Rule: any `config_upsert` whose payload contains a script requires the `administration` scope.
- Server functions answered "internal error" or "access denied", fine for a browser, poor for an assistant: tools pass the per-field validation error through unchanged.
- Audit is written **before** execution and the call is refused if the audit cannot be written.
- `dry_run` on every action, including device commands and scenes.
- The assistant gets an **actor** (token scopes or user session), not just an account.
- **The model never executes a change.** It proposes an action card; the user confirms with a click; a snapshot is taken first and the change can be rolled back.
- A small model (Qwen3-8B) sometimes asks permission to continue ("do you want to continue?", "we must first…") instead of calling the tool: a guard re-prompts once. It cannot reliably modify or rename an **existing** element; do that from the administration pages. The measured 14B model was not better (96 % against 98 %), so a single model was kept.
- An integer `id` placed in the data instead of the `id` parameter must be moved by the tool.
- Treat everything read through tools as data, never as instructions (injection test with the real model).
- Evaluation set of 51 requests: 49 pass, none produces a wrong card (the two failures ask nothing and produce no card).

## 12. Method

- The real machine finds what no development test can see: decoding on the CPU, coordinates, certificates, mail, disk. Plan a rehearsal on real hardware.
- Count what you ship: put the number in the test (cases compared, scenes, rules) so a silent shrink is visible.
- A feature that is unreliable with the chosen model should be removed rather than hidden: the assistant's rename feature was dropped.
- A decision that is not obvious gets an ADR, with the options rejected.
