# 0008 — ONVIF: a minimal hand-written SOAP client

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go** (with the reservations below)

## Context
No mature, suitable ONVIF crate exists. What is needed: WS-Discovery, identity/capabilities/profiles, RTSP URI, PTZ (`ContinuousMove`/`Stop`/`GetStatus`, presets), imaging (`IrCutFilter`). The spike (`reqwest` + `roxmltree` + `sha1`/`base64` + `socket2`) ran against 4 real cameras.

## Results
| Camera | Firmware | ONVIF | Result |
|---|---|---|---|
| 192.168.1.60 Jooan Q3R | Thingino stable | Device/Media/PTZ/Imaging | all OK; real PTZ pan (0 → 46.4°); IR-cut `Auto → ON → Auto` |
| 192.168.1.42 Jooan A2R | Thingino stable | same | all OK; PTZ 0 → 46.0°; IR-cut OK |
| 192.168.1.30 Xiaomi | Thingino master | Device/Media only | profiles + RTSP OK; no PTZ or imaging (expected) |
| 192.168.1.50 Ascam (Vatilon PB1) | vendor firmware | Device/Media/PTZ/Imaging | all OK; IR-cut `ON → OFF → ON`; service addresses on `:80/onvif/Media`… |
| 192.168.1.29 "NVT IPC" (`IPC_NT98566`, ONVIF 16.12) | NVT firmware | Device/Media/PTZ/Imaging on **`:8899`** (`/onvif/Media`, `/onvif/PTZ`, `/onvif/imaging`) | auth OK (`admin`); profiles 4096×2160 and 704×576; IR-cut `AUTO`; **real PTZ confirmed** by image comparison (the camera turns), but **`GetStatus` always returns 0,0**: no readable position; mechanical stops (a long `ContinuousMove` takes it to the end of travel); the camera's own web interface is unusable (plugin too old): everything goes through ONVIF/RTSP |
| 192.168.1.38 OpenIPC | OpenIPC | **no ONVIF** | the 401 comes from the web interface: control through the OpenIPC API (`openipc` night backend) |

- Authentication **WS-Security UsernameToken, PasswordDigest** (`base64(sha1(nonce + created + password))`) accepted by the three families; the camera clock is read first with `GetSystemDateAndTime` (no auth) to correct `Created` (measured offset: 0 s here).
- Unicast latencies: 12–110 ms per call (Thingino ≈ 15–50 ms, Ascam ≈ 50–90 ms).
- **WS-Discovery**: from the host, 4 devices found in 4 tries with 3 probes 1 s apart and 6 s of listening (a single probe / 4 s: 3 of 4 once). The Q3R **never answers** multicast despite a working unicast ONVIF → manual add by IP (`add-manual`) stays indispensable. An unlisted device (`192.168.1.29`, port 8899) also answered.
- PTZ: speed is not proportional to `x` on Thingino (≈ 100°/s as soon as non-zero); the accuracy of a movement depends on the delay between `ContinuousMove` and `Stop` (≈ 20–100 ms of HTTP jitter) → ± 3° after a round trip. `GetStatus` returns the position in degrees on Thingino.
- **`GetStreamUri` of the `.29` camera returns a URL that contains the user name and password in clear** (`rtsp://…/user=admin_password=…_channel=1_stream=0…`). Never log or display this value; always mask it (`password=***`).
- The ONVIF service port is not always 80 (here 8899, spotted by WS-Discovery through `XAddrs`): the service URL is a camera parameter, discovery fills it in.

## Decision
- `dominet-surveillance::onvif`: a hand-written SOAP client (envelope written by hand, answers parsed with `roxmltree`, chosen over `quick-xml` for its read-only DOM), limited to the calls used: `GetSystemDateAndTime`, `GetDeviceInformation`, `GetCapabilities`, `GetProfiles`, `GetStreamUri`, PTZ (`ContinuousMove`, `Stop`, `GetStatus`, presets), imaging (`Get/SetImagingSettings`).
- **Always** authenticate with WS-Security digest with clock correction; never display or log passwords.
- **An HTTP client without connection reuse** (`pool_max_idle_per_host(0)`): Thingino's ONVIF server closes the connection after each answer; reuse causes `connection closed before message completed` on the next call.
- Service addresses come from `GetCapabilities`, never deduced from the host (the Ascam uses `/onvif/Media`, Thingino `/onvif/media_service`).
- Discovery: 3 probes 1 s apart, 6 s of listening, interface chosen by the default route; a `discovery` role in host network.
- Contract tests: SOAP answers of these cameras captured (without secrets) in `tests/fixtures/onvif/`.

## Consequences
- Smooth PTZ control goes through `ContinuousMove` followed by `Stop` when the key is released, not through time-calibrated movements.
- Cameras without ONVIF (OpenIPC) need a dedicated backend (`NightControl::openipc`) and manual entry of parameters.
- Not covered: ONVIF events, `SetSystemDateAndTime`, HTTP Digest authentication (useful for the `goform` backend).
