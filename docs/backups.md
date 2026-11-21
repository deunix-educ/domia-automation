# Backups and restore

## What is backed up
| Source | File | Content |
|---|---|---|
| PostgreSQL | `<BACKUP_DIR>/postgres/dominet_YYYYMMDD_HHMMSS.sql.gz` | everything: accounts, settings, devices and their history, scenes, automations, cameras, indexed recordings, alert rules… |
| ReductStore | `<BACKUP_DIR>/reductstore/reductstore_YYYYMMDD_HHMMSS.tar.gz` | data volume: archived video segments, snapshots and detections if any, raw IoT |

Each file has its SHA-256 checksum (`….sha256`, checkable with `sha256sum -c`). **Keep the `DOMINET_MASTER_KEY` separately** — without it, camera passwords and SMS keys of a restored database are unreadable. Not backed up: segments still on disk before archiving, local snapshots, annotated images, generated Grafana dashboards, Prometheus and Grafana volumes (recomputable or short-lived data).

## Automatic backup
Every day at `BACKUP_HOUR` (3 am, local time), if `BACKUP_DIR` is set (`scheduler` role). Backups older than `BACKUP_RETENTION_DAYS` days (7) are deleted afterwards; **the newest of each kind always stays**. A failure is logged ("sauvegarde echouee"), counted (`dominet_backup_runs_total{result="error"}`) and triggers the `BackupFailing` alert; `BackupTooOld` warns when no backup has succeeded for 36 h.

## By hand
```bash
dominet backup      # immediate backup (database + ReductStore), prune expired ones
dominet backups     # list
```
In the deployment: `make prod-backup` (lot 6).

## Restore
**1. The database** (it **replaces** the current data; a single transaction: all or nothing):
```bash
dominet restore --latest postgres --yes             # the newest
dominet restore /backups/postgres/dominet_20261109_030000.sql.gz --yes
```
The checksum is verified first. Without the tool: `gunzip -c dominet_….sql.gz | psql -v ON_ERROR_STOP=1 --single-transaction` (if the server is PostgreSQL 16 and the dump was made by a 17 client, remove the `SET transaction_timeout` line: `| grep -v '^SET transaction_timeout'`).

**2. ReductStore** (ReductStore **stopped**, its data folder empty):
```bash
dominet restore --latest reductstore --to /path/to/reductstore/volume
# then start ReductStore on that folder
```
A non-empty folder is refused (`--force` to override).

**3.** Restart DomiNet with the original `DOMINET_MASTER_KEY`.

## Checking the procedure
`tools/check-backup.sh` (after `make dev-up` and `make image`) replays everything with the Docker image: a throwaway database backed up then restored into another one and compared table by table; real ReductStore records backed up, restored into another folder and read back byte for byte by a second ReductStore; a damaged file refused. [ADR 0054](adr/0054-backups.md).
