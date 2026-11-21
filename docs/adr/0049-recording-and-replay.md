# 0049 — Camera recording, ReductStore archive and replay

- Status: accepted
- Date: 2026-11-08

## Context
The reference had MediaMTX record (`record: true` on the *path* of cameras in continuous or motion mode, or after a manual start), scanned `/recordings` every 60 s, created a `Recording` per segment, pushed the whole file to the ReductStore bucket `surveillance` (entry `cam_<uuid>`, labels `event_type`, ASCII `camera_name`, `camera_uuid`, a 50 GB FIFO quota) and served the local file, failing that ReductStore. The local file was **never deleted** (the volume grew without limit) and the last file of each camera was always ignored. Criterion: "1 hour of continuous recording without losing a segment".

## Decision
- **MediaMTX records, DomiNet writes no video.** `PathConf` already carries `record` (continuous and motion: true); the format (`fmp4`), the path (`/recordings/%path/%Y-%m-%d_%H-%M-%S-%f`), the segment duration and the part (`1s`) are MediaMTX's **global** configuration (development: 10 s; production: 60 s).
- **Scan** (`dominet_surveillance::recording`): a segment is **finished** when its file has not been modified for `RECORDINGS_GRACE_SECS` (15 s). Deliberate deviation: the reference always ignored the last file, so the last segment of a camera whose recording stops was never taken; here the modification date is enough, all segments are. Start date read from the name (MediaMTX's `%f` = microseconds; read by hand), failing that the modification date.
- **`video` role** (`dominet_server::recordings`): every `RECORDINGS_SCAN_SECS` — indexes the finished segments of **active cameras that record** (mode ≠ `none`), then archives. Segment type: `manual` in manual mode, `motion` if a motion falls in its window (`RECORDINGS_SEGMENT_DURATION`), otherwise `continuous`. A segment already indexed (unique path in the database) is never indexed twice.
- **Archive** (`dominet_store::video::ReductVideo`): bucket `surveillance` (`REDUCTSTORE_VIDEO_BUCKET`), entry `cam_<uuid>`, timestamp in microseconds, labels as above (NFKD then ASCII for `camera_name`), FIFO quota `VIDEO_QUOTA_GB` at bucket creation. **Deliberate deviation: the local file is deleted after a successful push**, the volume no longer grows. A failing push (ReductStore down) notes the error on the segment (`status = error`), keeps the file and **retries at every pass**; without `REDUCTSTORE_URL` segments stay on disk (`pending`) and are served from it.
- **Replay**: `GET /surveillance/recordings/<uuid>/<id>/stream/` (any signed-in account). Local file if it still exists (`tower_http::ServeFile`: `Range`, native seek), otherwise the segment is read from ReductStore and served **with the same `Range` support** (206, `Content-Range`, 416). A path in the database outside `RECORDINGS_DIR` is never served. Page `/surveillance/recordings/<uuid>`: one day at a time (server zone, local hours), a native `<video>` player, previous / next; **deletion: administrators** (file, ReductStore entry, row).
- **Manual recording**: `PATCH /v3/config/paths/patch/<uuid>` `{"record": bool}` (`MediaMtx::set_record`), a Start / Stop button on the recordings page (administrators, camera in manual mode only). A full MediaMTX resynchronisation (start-up, "Synchronise" button) puts the path back to its base state — so it **stops** a manual recording in progress.
- **Deleted camera**: its segment folder is deleted with it, after its *path* is stopped.
- **Security**: paths served or deleted must be under `RECORDINGS_DIR` without `..`; folder identifiers are known camera UUIDs.

## Verification
- Unit: segment name reading (with / without fraction, microseconds), cooled segments only, last segment of a stopped camera taken, unreadable name, `Range` ranges (start, end, suffix, out of bounds), previous / next days (leap years).
- Integration (real ReductStore of the development stack, throw-away bucket, real database): indexing by mode, type `motion` / `continuous` / `manual`, push, **local file deleted**, reread from ReductStore with the exact labels, no duplicate at the next pass, deletion; ReductStore down then back (file kept, error noted, resumed); without archive; path outside the folder refused; manual recording (fake MediaMTX, refusal outside manual mode).
- End-to-end (`tests/e2e/recordings.py`, real H.264 stream → development MediaMTX → ReductStore): the path records, segments archived, 200 / 206 / 404, MP4 signature, size matching the index, page, player, manual mode (start and stop, `manual` segment), deletion.
- **Endurance** (`tests/e2e/recordings_soak.py`, `SOAK_SECONDS=3600 tools/e2e.sh recordings_soak`): one hour of continuous stream, no gap between consecutive segments, none in error, cumulated duration ≥ 97 % of the published duration, all reread from ReductStore. **Result: 3,600 s of continuous stream, the 7 checks pass.**

## Limits
- No 24-hour timeline: list + player. Postponable.
- `motion` mode records continuously (only the type distinguishes segments with motion); a motion-triggered recording would be a change of behaviour, to decide.
- The `/recordings` volume must be shared by MediaMTX and the `video` role, and writable by the latter (deletion). In development, MediaMTX runs under the user's identifier (`DEV_UID`) so files belong to them.
