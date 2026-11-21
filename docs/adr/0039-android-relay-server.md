# 0039 — Android relay, server side: endpoint, presence, orders and events

- Status: accepted
- Date: 2026-11-07

## Context
The Kotlin application (unchanged) opens `/ws/android/device/<token>/`, sends `sms_received`, `call_event`, `sms_sent_ack`, understands `auth_ok`, `send_sms`, `pong` and ignores the rest. The schema and protocol are laid down (ADR 0037); the endpoint, the "online" presence, order sending, and a way to notify browsers are needed.

## Decision
- **Exact endpoint**: `/ws/android/device/{token}/`, with no cookie or `Origin` (the application sends none). Malformed or unknown token: the WebSocket opens then closes with code **4001** (the reference refused before accepting; for the application, which reconnects after 5 s in both cases, it is identical). A malformed token does not touch the database. **The token is never logged**: the path is masked (`/ws/android/device/***/`) in access logs and CSRF refusals (`web::loggable_path`).
- **One connection per device**: a new one replaces the old one (closed with 4009). "Offline" is only set when the **current** connection closes: a phone that reconnects before the old line is detected dead does not flicker offline.
- **Heartbeat**: a control frame every 30 s; 90 s without any frame received (OkHttp answers every ping by itself): close 4408. At start-up of the web role every device goes back offline (the reference left devices "online" after a hard stop).
- **Frames received**: read by `dominet_domain::android`; unreadable JSON, non-object or unknown type ignored; a frame over 64 KiB: connection cut; empty number: ignored; number, contact and body trimmed to the database bounds (30 / 100 / 5,000 characters) rather than refused. A JSON `ping` receives `pong`.
- **Orders** (`AndroidRelay`): `send_sms` puts a row in `sms_outbox` then pushes `send_sms {to, body, message_id}` where `message_id` is that row's identifier; device offline: refusal without recording anything; a row sent into the void: marked failed at once. `push_alert` sends `alert_notification` to **all** connected devices (the current application ignores it). An acknowledgement only counts for the `pending` row **of its device**.
- **Acknowledgement delay**: a task sweeps every 10 s the rows without acknowledgement for 60 s and fails them ("no acknowledgement from the device"); a late acknowledgement has no effect. For a messaging recipient, the acknowledgement (or failure) updates its SMS status then the message state.
- **Browsers**: topic `android:<id>` of the existing WebSocket (`WsServerMsg::Android { device_id, event }`, events `device_status`, `sms_received`, `call_event`, `sms_sent_ack` in the reference's message format). Presence is **retained** by the hub (a subscriber receives it at once). Subscription reserved to the owner or an administrator; an unknown device and another's device get the same refusal. No `/ws/android/browser/<id>/` (no external client).
- **Call in progress**: for `answered` or an unknown state nothing is written but browsers see the open call (like the reference); the empty call the reference broadcast when no call exists is not rebroadcast.

## Verification against the reference
The reference consumer was run once on 21 sessions of the application; `tests/fixtures/android/oracle.json` keeps the frames returned, the state of SMS and calls, and the events broadcast; `crates/dominet/tests/android_oracle.rs` replays them against the Rust server and requires the same results. Known gaps, outside these sessions: the most recent open call closed, an empty number ignored, **the reference consumer crashes on JSON that is not an object** (`[1]`, `42`) whereas Rust ignores it, acknowledgements matched through the queue (the reference broadcast any `message_id`, even unknown; its identifiers collided), dates without a zone not handled (the application always sends an offset).

## Consequences
- The Android pages and messaging only have to call `AndroidRelay::send_sms`, read the `android:<id>` topic and the database.
- One device per connection; if a second phone used the same token they would disconnect each other in a loop (every device has its own token).
- The 60 s delay and the heartbeat are constants (overridable in tests through `AndroidRelay::new(Timing)`).
