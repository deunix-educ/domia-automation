# 0013 — CI on a self-hosted runner

- Status: accepted, then **partly replaced by [ADR 0015](0015-local-gate.md)** (CI is no longer automatic: local `pre-push` gate). The runner design is kept here as history; the public repository has no CI workflow and no runner scripts.
- Date: 2026-11-01

## Context
CI on GitHub's machines was slow (cold runs of several tens of minutes, caches to restore/export, a Docker image rebuilt without layers) and subject to a minute quota (about 2,000 a month on a private repository, free plan). The developer wanted fast CI without a quota. The same checks already run locally (`make ci`).

## Options
1. **GitHub machines** (status quo) — no maintenance, but slow, capped, and remote caches to manage.
2. **Self-hosted runner on the development machine, without isolation** (the machine's Docker, the developer's account) — fast, but a compromised workflow or third-party action would have root-equivalent access (`docker` group) on the production server.
3. **Self-hosted runner, isolated** (dedicated user without sudo, rootless Docker, capped resources) — chosen at the time.
4. **Another dedicated server** — better separation from production; still possible later (same install script).
5. **No remote CI, `make ci` alone** — fast and free, but no check on a clean machine and no per-commit trace.

## Decision (at the time)
- Option 3, **this private repository only**. The install script set everything up idempotently: a locked `gha-runner` user outside the `sudo`/`docker` groups, rootless Docker for it alone, Rust and tools installed once, runner verified by SHA-256, a user systemd service with automatic restart, weekly Docker cache cleanup.
- **Resource caps** on everything the runner launched: CPU weight 20, memory 4 GB "high" and 5 GB maximum.
- **The workflow ran `make ci`**: a single CI definition, identical locally and on the runner; no third-party action (only the official `actions/checkout`). The test database was a throw-away PostgreSQL container per job.
- Caches **on the runner**: a persistent Cargo target, Docker layers of the rootless Docker, pre-installed tools.
- Guard: pull requests from a **fork** do not run on this runner. No fallback to GitHub's machines.
- The emulated arm64 job was **removed** (slow and greedy); to resume with a native arm64 runner.

## Consequences
- CI became fast (warm caches: a few minutes) and no longer used GitHub minutes.
- It shared the machine with everything else; the runner was throttled, but a full build remained a load peak.
- A runner executes repository code: never use it for a public repository or fork PRs, never store secrets on it.
- Superseded in practice by ADR 0015.
