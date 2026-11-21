# 0044 — MediaMTX client and camera administration

- Status: accepted
- Date: 2026-11-07

## Context
The reference created a MediaMTX *path* per active camera through the REST API **and** rewrote `mediamtx.yml` so the configuration survived a restart. Decisions needed: secrets, identifier, synchronisation, rights.

## Decision
- **`dominet-surveillance::mediamtx` client** (reqwest, 5 s timeout): paginated list, read, `add`, `replace`, `remove`, and `sync` (create or replace each wanted path, delete the paths of **our** cameras that are no longer wanted). Only the names of **32 hexadecimal characters** are ours: the `~.*` of the configuration and any other path are never touched.
- **`replace` rather than `patch`**: it replaces the whole configuration, so an option removed from a camera (forced TCP, recording) disappears from the path. Checked on MediaMTX 1.21.3: `replace` **creates** the path if missing, `delete` of an absent path answers 404 (treated as a success).
- **No rewriting of `mediamtx.yml`**: the API alone. A restarted MediaMTX loses the paths added live; the database is authoritative and the web server reruns a full synchronisation at start-up (and the administration button on demand). In production, the YAML is generated from the database by the deployment. *Known limit*: if MediaMTX restarts alone, its cameras are missing until the next synchronisation.
- **Source = secret**: the source URL contains the camera's password. `PathConf` does not show its source in `Debug`, MediaMTX errors are cleaned of it, and nothing is logged.
- **`CameraService`** (server): encrypts secrets with the associated data `camera:<uuid>:<field>` (the public identifier is drawn **before** insertion, since it binds the encryption), keeps a secret left empty on modification, reuses the RTSP credentials for ONVIF if missing, keeps what ONVIF synchronisation learned (manufacturer, model, profile, PTZ). A camera whose secret is unreadable (master key changed) is not removed from MediaMTX by mistake.
- **A MediaMTX failure does not undo the save**: the database is authoritative, the failure is logged and `sync_all` catches up.
- **Configuration**: `MEDIAMTX_API_URL`, optional; without it nothing is synchronised and the administration says so.
- **Rights**: `/surveillance` (viewing) for any signed-in account; `/admin/cameras` (CRUD, synchronisation) and the matching server functions for administrators. The viewing page exposes neither IP address, path nor credentials.
- **Fake MediaMTX** (`dominet_surveillance::testing`, feature `testing`): the configuration API in memory, with simulated failure, modelled on the verified behaviour of the real one. A test **ignored by default** (`real_mediamtx`) replays the client against the development stack's MediaMTX.

## Consequences
- Live view, snapshots, ONVIF and PTZ are added on these same cameras.
- A camera password can only be entered with a configured master key.
