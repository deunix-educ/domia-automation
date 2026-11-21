# Android relay (server side)

R5 lot 3, [ADR 0039](adr/0039-android-relay-server.md) (model: [ADR 0037](adr/0037-android-relay-and-messaging.md)). The Android app is **not modified**: the Rust server speaks exactly its protocol.

## What the app does

It connects to `wss://<server>/ws/android/device/<token>/` (the device token, shown only once when created or regenerated) and reconnects by itself after 5 s.

| Direction | Frame | Effect |
|---|---|---|
| app → server | `sms_received {from, body, contact_name, timestamp}` | SMS stored (unread), shown live |
| app → server | `call_event {state, number, contact_name, timestamp, duration}` | call log (`ringing` / `outgoing` create, `ended` / `missed` close, `answered` changes nothing) |
| app → server | `sms_sent_ack {message_id, success, error}` | the requested SMS is "sent" or "failed" |
| server → app | `auth_ok {device_name}` | connection accepted; the device is "online" |
| server → app | `send_sms {to, body, message_id}` | sends an SMS; the app answers with `sms_sent_ack` carrying the same `message_id` |
| server → app | `pong`, `alert_notification {…}` | (the current app ignores the latter) |

Unknown token: close with code **4001**.

## Presence and heartbeat

The server sends a control frame every 30 s; with nothing received for 90 s it closes (code 4408). A new connection from the same device replaces the old one (4009). At startup every device goes "offline".

## SMS to send

`AndroidRelay::send_sms` queues the SMS (`sms_outbox`) and sends it to the device; the ack settles it. **Without an ack after 60 s the SMS fails** ("no ack from the device"). An ack from another device, for an unknown id, or late, has no effect.

## For browsers

Topic `android:<id>` of the notifications WebSocket (owner or administrator): `device_status`, `sms_received`, `call_event`, `sms_sent_ack`. The current presence arrives on subscription.

## The pages (lot 4, [ADR 0040](adr/0040-android-pages.md))

| Page | Content |
|---|---|
| `/android` | devices (live presence), last 10 SMS and calls, unread SMS count |
| `/android/devices` | add a device (name, number), **new token**, delete (with confirmation); **the token is only shown at creation or regeneration** |
| `/android/sms?device=N` | the last 100 SMS (received ones are marked read when displayed), send an SMS; each outgoing SMS shows its state: waiting, sent, failed and why |
| `/android/calls?device=N` | the last 100 calls (incoming, outgoing, missed, duration `mm:ss`, "ongoing") |

Each account sees its devices, an administrator sees all devices. Everything updates live (received SMS, calls, acks, presence). Address to enter in the app: `ws://server-address/ws/android/device/TOKEN/` (`wss://` behind HTTPS).

## Security

- The token is only stored hashed (SHA-256) and **appears in no log** (the path is masked).
- Frames limited to 64 KiB; fields clipped to the database bounds; invalid JSON ignored without closing the line.

## Tests

`cargo test -p dominet --test android_ws` (15: token, presence, replacement, heartbeat, SMS, calls, acks, messaging, timeout, alerts, subscription rights), `--test android_oracle` (**21 sessions replayed against frozen reference exchanges**), `-p dominet-domain --test android_fixtures` (frames from the Kotlin code), `--test ui android` (rights, token, sending), E2E `android` (simulated phone `tools/sim_android.py`).
