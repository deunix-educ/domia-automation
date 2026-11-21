# ADR 0054 — Backups and restore

Status: accepted.

## Context
A nightly backup (3 am) of the database and of the ReductStore volume, kept 7 days, is worth only what a restore can recover: what matters is being able to **restore**, not only to back up.

## Decisions
- **Database: `pg_dump`** (SQL, `--clean --if-exists --no-owner --no-privileges`), gzipped, written under a temporary name then renamed (never a half-written file). Standard format: restorable by hand (`gunzip -c f.sql.gz | psql`). The client is in the image (`postgresql-client`).
- **ReductStore volume: a tar.gz archive** of the data folder mounted **read-only** (`REDUCTSTORE_BACKUP_SRC`). Accepted limit: consistency is "best effort" while ReductStore writes (it writes immutable blocks then an index); the restore was **verified with real records read back by a second ReductStore**.
- **Integrity check**: each file has its SHA-256 sum beside it (`*.sha256`, readable by `sha256sum -c`); a restore verifies it **before** touching anything.
- **Database password**: passed to the PostgreSQL clients through the child process environment (`PGPASSWORD`), **never** on the command line (visible in `ps`).
- **Restore**: `dominet restore <file>|--latest postgres --yes` replays the SQL in **a single transaction** (`ON_ERROR_STOP`): a failure does not leave a half-restored database; `--yes` is mandatory (it replaces the data). `restore --latest reductstore --to <folder>` extracts the archive (ReductStore stopped; a non-empty folder is refused without `--force`; paths that escape the target folder are ignored).
- **Line dropped at restore**: `pg_dump` 17 (Debian 13's, in the image) writes `SET transaction_timeout = 0;`, a parameter **unknown to PostgreSQL 16** (the stack's server): the restore would fail. Observed on the real binary; the single `SET transaction_timeout…` line is removed from the stream (tested).
- **Retention**: `BACKUP_RETENTION_DAYS` (7), applied after each backup; **the most recent of each kind is never deleted** (a long stop of the job must not leave nothing).
- **Scheduling**: in the scheduler (`scheduler` role), every day at `BACKUP_HOUR` (3, local time, daylight-saving handled), active only if `BACKUP_DIR` is set. `dominet backup` triggers it by hand, `dominet backups` lists, `dominet restore` restores.
- **Monitoring**: metrics `dominet_backup_runs_total{result}`, `…_last_success_timestamp_seconds` (files re-read at start-up), `…_last_duration_seconds`, `…_last_size_bytes{kind}`; shipped rules `BackupTooOld` (over 36 h) and `BackupFailing`. Without `BACKUP_DIR` the metric does not exist and nothing alerts.

## Not backed up — good to know
- Encrypted secrets (camera passwords, SMS keys) are only readable with the **original `DOMINET_MASTER_KEY`**: keep it **apart** (a database backup without it loses those secrets).
- Video segments still on disk (before archiving), snapshots, annotated images, generated Grafana dashboards and the Prometheus / Grafana volumes are not backed up: they are recomputable or short-lived data; the Compose files say which ones are volumes.

## Checks
- Integration tests (`tests/backup.rs`, `pg_dump` of the dev container): backup → restore into **another** database → fingerprint of **every table** and the schema identical; accents and dashes kept; restore over an existing database; a damaged file refused without touching the database; a failing client leaves no residual file; retention; volume archive (round trip, `../` path refused).
- `tools/check-backup.sh`: with the **image** (client 17) against PostgreSQL 16 and real throw-away ReductStores: same checks, plus a byte-by-byte reread of records by a second ReductStore.

## Consequences
- The restore procedure is in `docs/backups.md`.
