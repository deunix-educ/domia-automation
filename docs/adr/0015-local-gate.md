# 0015 — Local gate (pre-push hook) instead of automatic CI

- Status: accepted — replaces the "automatic CI on a runner" part of [ADR 0013](0013-self-hosted-ci.md)
- Date: 2026-11-01

## Context
ADR 0013 moved CI to a self-hosted runner on the development machine (free, no quota). But the workflow only reruns `make ci` (plus the interface build and an image test), which the developer already runs before every commit on the same machine, which is also the development and production server. Real cost: 8 to 10 minutes per push, a dedicated system user, rootless Docker, resource ceilings to maintain, and a recent failure (red) impossible to read or reproduce without access to the GitHub API. For a one-developer project, a per-commit trace brings nothing.

## Options
1. **Keep automatic CI** (ADR 0013) — conventional, but slow, heavy and redundant here.
2. **Nothing but `make ci` by hand** — simplest, but does not catch a file forgotten in `git add` (everything compiles in the working tree, nothing is checked on what will really be pushed).
3. **`pre-push` hook: `make ci` on a clean copy of the pushed commit** — chosen.
4. **GitHub CI triggered on demand only** — see Decision.

## Decision
- `.githooks/pre-push` (enabled by `make hooks`, which sets `core.hooksPath`) extracts each pushed tip commit with `git archive` (so **without untracked files or uncommitted changes**), starts a throw-away PostgreSQL (`postgres:16-alpine`, random port, removed at the end) and runs `make ci` at low CPU priority (`nice`). On failure nothing is pushed; the last 60 lines and the log path are shown.
- A dedicated Cargo target (`target/prepush`) does not interfere with the build in progress; the first run is longer.
- `make ci-full` = `make ci` + `make front` + `make image-smoke` (interface build and Docker image), to run before a milestone and when the `Dockerfile` or the build chain changes. `DOMINET_PREPUSH_CMD='make ci-full' git push` applies it to the push.
- Bypass: `git push --no-verify` or `DOMINET_SKIP_PREPUSH=1`. It is a safeguard, not a barrier: a local hook does not protect against a deliberate bypass.
- The public repository keeps **no GitHub workflow**; the gate is `make ci`, run locally and by the hook.

## Consequences
- A push takes the time of `make ci` on a clean copy (a few minutes, less when warm) instead of 8 to 10 remote minutes.
- Tests run against a throw-away PostgreSQL only: never a real Mosquitto, cameras or Zigbee dongle. They only consume CPU (`nice`).
- No CI status on GitHub: the proof that a commit passed the checks is that it could be pushed. If the project gains contributors, going back to automatic CI (ADR 0013, or GitHub-hosted machines) becomes relevant again.
