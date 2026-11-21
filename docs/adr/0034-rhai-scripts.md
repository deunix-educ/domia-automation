# 0034 — Rhai scripts: implementation (automations and source handler code)

- Status: accepted
- Date: 2026-11-02

## Context
ADR 0006 chose sandboxed Rhai (spike go). This work integrates it into the automation engine (ADR 0032) and opens it to the `handler_code` of MQTT sources, with an editor and a dry run.

## Decisions
- **Host in `dominet-automation::script`**, as in ADR 0006: engine rebuilt at every execution, `new_raw` + the standard package, 200,000 operations, 250 ms, depth 24 / 48, strings 16 KB, arrays 2,000, maps 500, no `eval` or `import`, execution in `spawn_blocking`. **Deviations from ADR 0006**: limits are constants (`Limits::default()`, passed as a parameter) and not `AppSetting` settings — nothing justifies them in administration yet; no AST cache (compilation in the order of ten microseconds, negligible against the database).
- **All or nothing.** Effects (`publish`, `scene`, `log_value`) are collected then applied in order **only if the script ends without error**: a script that publishes then crashes does not leave a half-applied state (the reference applied as it went). An effect refused at application (broker down, unknown scene) does not stop the following ones and makes the execution fail in the log.
- **Missing key = `()`.** ADR 0006 made access to a non-existent key fail; with `payload.moving == "UP"` on a state that has no `moving` (a common case: Zigbee2MQTT states are partial), it was an error at every press. Access returns `()`; a typo becomes silent, `print` helps find it.
- **The `Executor` port grows**: `publish` takes a text, and `scene_by_name`, `log_value` and `devices` (a snapshot of active devices, read only when an action is a script) are added. Scripts therefore go through the same engine, the same guard and the same log as other actions.
- **Trigger context**: the event carries the topic (the device's for a state), the message and the device concerned; a schedule or solar trigger gives an empty context.
- **Source handler code.** Ingestion announces (`Sink::source_handler`) the messages of active sources that have code; the engine runs them in the same queue. No automation so no `automation_run` row: the result goes to the server log. **A 10 s mute** for a source whose code fails: a chatty source with faulty (or hostile) code does not monopolise the engine (at worst 250 ms every 10 s). Correct code is not limited in count (a source like Linky sends a message per second).
- **Code in another language refused with a clear message.** At save time, `script::check` recognises Python-style code (`def`, `import`, `from … import`, `class`, `elif`, `context.`, `logger.`, trailing colon) before trying to compile, then returns the Rhai parser's error (line, column). The field shows `Invalid script: …` (`ProblemKind::Syntax`).
- **Editor**: a fixed-font code area (`FieldKind::Code` of the generic CRUD) and a **dry run** (`scripts/dry-run`, administrators): same host, real device states, effects described and not applied. CodeMirror remains possible later without changing the API.
- **Shipped shutter scripts.** Two example scripts (`crates/dominet-domain/data/volet-sb-est-*.rhai`) stop a moving shutter on a press, otherwise open (left button) or close (right button); they depend on a specific relay's behaviour (position-derived state, about one message per second while moving) and are examples to adapt.

## Consequences
- The time limit only acts between two Rhai operations: any future host function must be non-blocking and bounded (today all are in-memory additions).
- A slow script (250 ms) delays the following events of the single queue; the bounded queue drops and counts what overflows.
- The hostile scripts of ADR 0006 are all replayed in unit tests, in dry run and in end-to-end tests (the server answers during and after).
