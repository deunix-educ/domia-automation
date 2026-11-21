# 0047 — PTZ control and day / night switching

- Status: accepted
- Date: 2026-11-08

## Context
The reference drives PTZ through a WebSocket (ONVIF `ContinuousMove` / `Stop`, presets in the database) and controls day / night, IR-cut and light through four methods (OpenIPC, Thingino, ONVIF, Goform). ADR 0008: the ONVIF client. Decisions: rights, methods tested against fake servers.

## Decision
- **ONVIF PTZ** (`dominet_surveillance::onvif::ptz`): `PtzControl` reads once the PTZ service address and the token of the **first profile that carries a PTZ configuration** (the one the spike validated on the cameras), then `ContinuousMove` (only the axes in motion are sent; all zero = a `Stop`), `Stop`, `GetStatus` (position, coordinate space, `MoveStatus`), presets (`GetPresets`, `SetPreset`, `RemovePreset`, `GotoPreset` at half speed) and `AbsoluteMove` in **the space announced by the camera** (Thingino: degrees; the default would be `[-1, 1]`), waiting for the end of the movement (8 s at most). Speeds are brought into `0.05..1` (an unreadable value never produces a movement).
- **WebSocket `/ws/surveillance/ptz/<uuid>/`**: the reference's **protocol**, unchanged (`move`, `zoom`, `stop`, `get_presets`, `set_preset`, `remove_preset`, `goto_preset`; answers `presets` and `error`, plus a `ready` the reference did not have). Deliberate deviations:
  - **administrators only** — the reference let any signed-in account in; refusal **before** opening (session, role, origin, unknown / inactive / PTZ-less camera);
  - **automatic stop** of a movement not refreshed (`PTZ_DEAD_MAN_SECS`, 8 s) in addition to the stop on close: a frozen tab does not leave a camera turning to the end stop (some have no software stop, see ADR 0008);
  - a preset **of the same name replaces** the old one and **removes the camera's old token** (the reference left it orphan); the database stays the source of truth; a preset without a token is reached by `AbsoluteMove` to its saved position.
- **Interface** (`/surveillance/live/<uuid>`, administrators): a direction cross (a **held** direction resends its order every 250 ms, `stop` on release, when leaving the button and if the pointer is interrupted), zoom, two speeds, presets; link resumed every 1.5 s if it drops. Buttons only appear if the camera has PTZ and a day / night method.
- **Day / night** (`dominet_surveillance::night`, `NightControl` trait, four methods):
  - `openipc`: `GET /night/<toggle|light>` (`daynight` and `ircut` go to `toggle`, which activates the right engine pin);
  - `thingino`: `POST /x/json-imp.cgi`, `X-API-Key`, toggle **remembered per camera and per action** (24 h: the camera does not reread its state);
  - `onvif`: `GetImagingSettings` → a single field changed → `SetImagingSettings` **of the complete object** (a partial object resets the other settings); `AUTO` and `DAY` go to `NIGHT`, `NIGHT` to `DAY`; the light toggles brightness between 0 and 80;
  - `goform`: JSON `{Type: 1, Data}`, `X-Digest` authentication (a challenge without body, then an MD5 answer with `qop=auth`); only `DayNightMode` (or `Control` for the light) changes, `ModeSupportList` is not sent back. Web credentials, otherwise the stream's.
  - Command through the `camera_night` server function: **administrators**, active camera only. No error contains a secret.
- **Verification**: a **stateful** fake ONVIF (position, speed, presets, image settings, `MOVING` delay), fake OpenIPC, fake Thingino (API key), fake Goform (the Digest challenge); the Digest computation is compared with a value **computed separately**; a real WebSocket end to end (rights, origin, speeds, automatic stop, presets, `AbsoluteMove` in the camera's space); a Firefox end-to-end test against a simulated ONVIF camera that logs every order (`tests/e2e/ptz.py`).

## Consequences / limits
- **`goform` has no test camera**: its conformity rests on the reference read, on the verified Digest computation and on a fake server; to confirm on matching hardware.
- One camera model never gives its position (`GetStatus` returns 0, 0): a preset saved on it has position 0, 0 and a camera token (it works by token, not by position).
- Trials on real cameras (PTZ, IR-cut) are done with an access file kept outside the repository; no order was sent to a real camera.
