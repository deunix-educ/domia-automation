# 0018 — Dashboard schema: overlap exclusion and references without foreign keys

- Status: accepted
- Date: 2026-11-01

## Context
`dashboard` and `widget` are introduced. Two difficulties: (1) two widgets of the same dashboard must never overlap, even when two moves arrive at the same time; (2) a widget designates a device, a scene or a camera, tables that do not exist yet.

## Options
**Overlap.** (a) Check in Rust before writing — not atomic: two simultaneous requests both pass. (b) An application lock alone — correct but does not prevent other code (CLI, migration, hand-written SQL) from breaking the invariant. (c) **A PostgreSQL exclusion constraint** (`EXCLUDE USING gist`, `btree_gist` extension) — the invariant lives in the database — chosen.

**References.** (a) Create minimal `device`/`scene`/`camera` tables now — encroaches on later work and freezes a schema not yet known. (b) **Columns without a foreign key** (`device_id`, `scene_id` integers, `camera_uuid` UUID), the key being added by the migration that creates the target table — chosen. (c) Store references in the `config` JSON — loses integrity and queries.

## Decision
- A 12-column grid; `grid_col`, `grid_row`, `width`, `height` (`col`/`row` are tricky words in SQL), all bounded by `CHECK`s (`grid_col + width <= 12`). A widget's rectangle is a `box(point(col, row), point(col + width - 1, row + height - 1))` in cell coordinates, bounds included: contiguous widgets pass, an overlap is refused (`widget_no_overlap`, SQLSTATE `23P01`, reported by the repository as `RepoError::Conflict`). Tested on the database (constraints) and on the repositories (races).
- **Widget writes serialised per dashboard**: the repository takes a row lock on the dashboard before inserting, modifying or moving. Without it, two simultaneous insertions on the same cells block each other in the constraint check (`deadlock detected`, documented PostgreSQL behaviour) instead of being cleanly refused; a race test (6 concurrent insertions, 10 rounds) brought it to light.
- **Default dashboard**: a partial unique index (`WHERE is_default`) = at most one per owner. The repository takes a lock on the owner's row, rereads the state *after* the lock (a read before it gave a stale value: the race revealed it), and moves the status from one dashboard to another in the same transaction. An account's first dashboard is its default; the status is not removed directly, it passes to another dashboard.
- Cascade deletion: user → dashboards → widgets.
- `presentation` (text, `''` = automatic), `config` (a JSON object of at most 4 KB) and `content` (HTML of the `text` widget, at most 20,000 characters) are bounded in the database **and** validated by the domain (`WidgetDraft::problems`, per field): consistency of type / presentation / references, Grafana configuration (`panel_url` http(s), or `dashboard_uid` + `panel_id`, or `key` with a device).

## Consequences
- The production database must be able to run `CREATE EXTENSION btree_gist` (a "trusted" extension since PostgreSQL 13, included in the `postgres:16` image).
- Later migrations add the foreign keys `widget.device_id`, `widget.scene_id`, `widget.camera_uuid` (`ON DELETE SET NULL`) and handle possible orphan references.
- A widget whose target disappeared keeps a reference to nothing while the key does not exist: the interface shows "source unavailable".
- HTML content is **not** sanitised by the repository: sanitising (`ammonia`) is done at save time, on the server.
