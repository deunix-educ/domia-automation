# 0020 — System statistics: `sysinfo`, published as the last state of the `system` topic

- Status: accepted
- Date: 2026-11-01

## Context
The reference implementation measured CPU, memory and disk every 5 s (a background thread) and every tab polled `/api/system-stats/` every 5 s. The sidebar gauges are kept, a temperature is added, and a WebSocket hub with last state kept is now available (ADR 0019).

## Options
**Source of measurements.** (a) Read `/proc` and `/sys` by hand — little code but fragile (formats, containers, other platforms); (b) `procfs` / `sysinfo`; (c) **`sysinfo`** (MIT, no `unsafe` on our side, a unified CPU / memory / disks / sensors API) — chosen.

**Broadcast.** (a) Tabs poll the API in a loop — N authenticated requests (session read in the database) every 5 s per tab; (b) **the collector publishes on the `system` topic** (last state kept) and tabs subscribe — one computation, one publication, a new tab served at the first beat — chosen. The REST API remains (clients without WebSocket, scripts) and **rereads the same last state**: no second cache, no on-demand measurement, so no extra load from an indiscreet client.

## Decision
- `dominet-server::system_stats`: `Sampler` (`sysinfo` instruments kept from one reading to the next, essential for CPU load which is a difference), reading **off the reactor** (`spawn_blocking`), first measurement after `MINIMUM_CPU_UPDATE_INTERVAL`, then every `SYSTEM_STATS_SECS` seconds (default 5, 1 to 3600).
- Conversion into `SystemStatsDto` by **pure** functions (`build`, `pick_temperature`), tested without a machine: GiB and percentages to one decimal, percentages bounded to 0–100 and never `NaN`, used memory = total − available, disk = `/` only.
- Format: the keys `cpu{percent,count}`, `memory{percent,used_gb,total_gb,available_gb}`, `disk{percent,used_gb,total_gb,free_gb}`, `updated_at` (Unix seconds) **plus `temperature`**. `disk` and `temperature` are `null` when the machine does not expose them: a missing value rather than a wrong one. With no measurement at all (start-up, collector stopped), the API answers **503**, not zeros.
- Temperature: the CPU sensor first (`coretemp`, `package`, `cpu`, `tctl`, `k10temp`, `soc`), otherwise the hottest; values outside `0 < t ≤ 150` ignored.
- `GET /api/system-stats/` requires a signed-in account (401 otherwise), all roles.
- Browser: a `SystemGauges` component subscribed to the `system` topic; bars are `div`s whose **width is set through the CSSOM** (`style:width`), the only way compatible with the CSP without `unsafe-inline`; "…" values while no measurement is received or if the connection is cut (no stale figure).

## Consequences
- In Docker, `/proc` describes the host (CPU, memory) but the disk is the container's and the temperature needs `/sys/class/thermal` mounted; otherwise `null`.
- An always-active `system` topic costs one message per interval even without a subscriber (negligible: a few hundred bytes).
- `sysinfo` adds transitive dependencies: `cargo deny` and `cargo audit` check them.
