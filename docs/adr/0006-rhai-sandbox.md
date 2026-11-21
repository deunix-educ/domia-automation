# 0006 — Automation scripts: sandboxed Rhai host

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go**

## Context
Plugins and source-handling code run user code. In Rust, scripts go through an embedded **Rhai** engine. Requirements: an API `publish/get_device/scene/log_value`, variables `topic`/`payload`/`device`, limits (operations, depth, size, time) and a hostile script stopped without affecting the process. Spike: Rhai 1.27.0, features `sync`, `serde`, `no_module`.

## Results
- 18 checks, 0 failures: legitimate script (full API), unknown `get_device` → `()`, invalid topic refused.
- Hostile scripts all stopped in under 45 ms, stable memory (max RSS 5.6 → 6.7 MB): infinite loops, infinite recursion, string/array/map growth, `pad` to 2·10⁹, `eval`, `import`, 5,000-level nesting (rejected at parsing), 100+ `publish`.
- Time limit (`on_progress`) validated alone, operations unlimited: stop at 250 ms.
- Panic of a host function: isolated by a thread, the process survives.
- Performance: script compiled once (AST), about 1 µs per execution, about 850,000 executions/s (without the realistic JSON conversion cost, in the order of ten µs).

## Decision
- Rhai host in `dominet-automation`, **engine rebuilt per execution** (cheap) with: `Engine::new_raw` + `StandardPackage`, `set_max_operations(200_000)`, `set_max_call_levels(24)`, `set_max_expr_depths(48, 24)`, strings ≤ 16 KB, arrays ≤ 2,000, maps ≤ 500, `disable_symbol("eval")`, `no_module` (no `import`), wall-clock limit of 250 ms through `on_progress`. These values are tuning parameters (`AppSetting`), not frozen constants.
- **No direct side effect**: `publish`, `scene`, `log_value` append `Effect`s (at most 100 per run) that the engine applies afterwards; `get_device` reads a snapshot supplied by the host. `publish` topics are validated (no wildcard, ≤ 256 characters).
- `print` goes to the automation execution log; `debug` is ignored.
- Execution in `spawn_blocking` (or a dedicated thread): a panic in a host function becomes a logged execution error, not a process stop. ASTs are cached per automation (invalidated on edit).

## Consequences
- The time limit only acts between two Rhai operations: a heavy native function would not be interrupted. Bounded sizes (arrays, strings) and the absence of blocking host functions avoid it; any future host function must be non-blocking and bounded.
- Scripts written in another language do not port as is: rewrite them in Rhai.
