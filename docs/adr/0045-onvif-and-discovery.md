# 0045 — ONVIF, discovery and adding cameras

- Status: accepted
- Date: 2026-11-07

## Context
The reference speaks ONVIF with a Python library, discovers cameras through a small separate HTTP server in host network (port 8765) and **creates a record for every device found**, before the user has chosen. ADR 0008: a hand-written SOAP client. Decisions: discovery, no contact with a production installation.

## Decision
- **Client** (`dominet_surveillance::onvif`): `GetSystemDateAndTime`, `GetDeviceInformation`, `GetCapabilities`, `GetProfiles`, `GetStreamUri`. Answers read without regard to name prefixes (the four camera families use different ones). WS-Security `PasswordDigest` with **correction of the camera's clock** (read without authentication first); no connection reused (Thingino closes it).
- **Service addresses keep the registered camera's host**: only the port and path announced by `GetCapabilities` are taken. A camera behind address translation sometimes announces its own; above all, an answer must not be able to send us to another host.
- **Secrets**: `Credentials` and `StreamUri` show nothing in `Debug`. The NVT camera's stream URL contains the password in clear: `mask_secrets` hides it by pattern (`user:***@`, `password=***`) **and** by known value (a password containing `_` would otherwise be cut too early); only the masked form leaves the module.
- **Synchronisation** (`onvif_sync`): the first profile is the main profile and PTZ = presence of the PTZ service; ONVIF credentials, otherwise the stream's. Results are saved without touching entered settings.
- **Adding by address** (`add_by_ip`): the ONVIF port is searched among 80, 8080, 8899, 2020, 8000, but **an open port is not enough**: it must answer `GetSystemDateAndTime`. An already known address returns the existing record. With credentials, the camera is synchronised at once (a failure does not cancel the addition).
- **Discovery (deviation from the reference)**: results are **listed with an "Add" button**, instead of creating a record per device. A device already registered at this address is flagged.
- **WS-Discovery**: 3 probes one second apart, 6 s of listening, outgoing interface by the default route, one announcement per address. The `discovery` role (host network) and the `web` role do not call each other: they go **through PostgreSQL** (`LISTEN`/`NOTIFY`, channels `onvif_discovery_request` / `onvif_discovery_result`), which replaces the reference's HTTP server without a new port. The `web` role relays results to the `discovery` WebSocket topic (administrators only), after marking addresses already registered. Without a `discovery` role: the interface says so after five seconds.
- **No discovery by default**: `dominet serve` does not start the `discovery` role (it needs host network); `dominet serve --only discovery`.
- **Verification**: a fake ONVIF server checks the digest and the clock like a camera (4 families of answers); a fake WS-Discovery responder answers unicast on loopback (CI never does multicast). The answers of `fixtures/onvif/` are **reconstructed** from the spike (no secret, no real address). Two tests **ignored by default** confront them with real cameras (`real_onvif`, read-only) when an access file is present.

## Consequences
- A camera that multicast does not see (one model) is added by its address.
- PTZ, imaging and the `onvif` night mode reuse this client.
- The `discovery` role needs database access: it uses it as a message channel.
