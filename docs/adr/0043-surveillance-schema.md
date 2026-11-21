# 0043 — Surveillance schema: cameras, recordings, events

- Status: accepted
- Date: 2026-11-07

## Context
Surveillance covers cameras, PTZ presets, recordings, snapshots, motion events, detection events and the detector configuration.

## Decision
- **Seven tables**, with `CHECK` constraints that repeat the domain's rules (ports, fps, priority, enumerations, confidence in [0, 1]): the database refuses what the domain refuses. Deleting a camera: everything that belongs to it goes in cascade; deleting a place: the camera is detached (`SET NULL`).
- **Public identifier**: `camera.uuid`, a `UUID` drawn by the database (`gen_random_uuid()`), displayed as **32 hexadecimal characters** (`CameraUuid`). It is the MediaMTX path name and the widget key (`camera_uuid`, without a foreign key). The numeric key stays the internal foreign key.
- **Secrets**: RTSP password, ONVIF password, web password and web API key are stored **encrypted** (`*_enc`, the `v1:` format of ADR 0011). The domain only sees a `Sealed` (already encrypted text, masked `Debug`): encryption and decryption are done by the server (`SecretBox`, associated data specific to the camera and the field), never by the repository. URLs that contain a password only exist in memory (`source_url`); `masked_source_url` is the only one that is displayed or logged.
- **Repositories**: one port per aggregate (`CameraRepo`, `PtzPresetRepo`, `RecordingRepo`, `SnapshotRepo`, `EventRepo` for motions and detections, `DetectorConfigRepo`). `Recording` follows `pending → uploading → done | error`; `list_unarchived` returns what remains to push (resume after a stop). `detector_config` is a singleton (`id = 1`) created by the migration.
- **Free fields**: the stream path refuses spaces and `@`, the host only allows letters, digits, `.`, `-`, `:` — they enter a URL passed to MediaMTX and `ffmpeg`.
- **Choices**: segment duration in milliseconds (`duration_ms`); YOLO classes as `TEXT[]`; detection labels as `JSONB` validated as an array; preset positions `position`.

## Consequences
- A camera imported from another system will have secrets to encrypt and a new identifier.
- Changing `DOMINET_MASTER_KEY` makes camera secrets unreadable, like those of settings (ADR 0011): the camera stays, the passwords must be entered again.
