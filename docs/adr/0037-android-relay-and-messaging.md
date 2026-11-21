# 0037 — Android relay and messaging: data model

- Status: accepted
- Date: 2026-11-06

## Context
The Android relay (a phone sends received SMS and call events, receives SMS to send) and internal messaging (one message, several recipients, e-mail and/or SMS) are needed. The tables `message`, `message_recipient`, `android_device`, `sms_message`, `call_log` exist from the first phase. The Kotlin application must not change.

## Options
- **Clear-text token**: a database leak gives access to all devices.
- **Token fingerprint**: only the SHA-256 fingerprint is stored; the clear value is shown at creation or regeneration only.
- **SMS acknowledgements**: reuse `sms_message.id` (browser SMS) and `message_recipient.id` (messaging) as the protocol's `message_id` — ambiguous, two identifier spaces.

## Decision
- **`android_device.token`** (UUID) is replaced by `token_hash` (hexadecimal SHA-256, 64 characters, unique). The token is produced by `gen_random_uuid()` **in SQL**, the fingerprint too: the clear value only travels in the creation / regeneration answer. A malformed token in the address (`canonical_token`) is treated as unknown.
- **`sms_outbox` queue**: an SMS requested from a device has its own identifier, which becomes the `message_id` of `send_sms` and comes back in `sms_sent_ack`. The origin is exactly one of: a browser SMS (`sms_message_id`) or a messaging recipient (`message_recipient_id`) (constraint `sms_outbox_one_origin`). An acknowledgement only changes the `pending` rows **of the device that sends it**: another device, an unknown identifier or a second acknowledgement have no effect. `pending` rows older than the delay (60 s) become `failed` (`expire`).
- **Call log** (`plan_call`, pure): `ringing` / `outgoing` create; `missed` / `ended` close the open call of the same number, or create an already finished entry; `answered` and unknown states do nothing. **Deliberate deviation from the reference**: the most **recent** open call is closed (the reference took the oldest).
- **Messaging**: states `pending` / `sent` / `failed` / `na` per channel and recipient (`initial_statuses`: `pending` only if the channel is requested and the contact detail exists); the message becomes `sent` when no send is pending any more (`finish_if_complete`, atomic).
- **Protocol** in `dominet_domain::android` (pure, tested against frames taken from the Kotlin code, `tests/fixtures/android/frames.json`): `parse_device_frame` applies the reference's defaults; unreadable JSON is ignored.
- **Ports**: `AndroidDeviceRepo`, `SmsRepo`, `CallRepo`, `SmsOutboxRepo`, `MessageRepo`; adapters `PgAndroidDevices`, `PgSms`, `PgCalls`, `PgSmsOutbox`, `PgMessages`.

## Consequences
- The migration is one-way: the clear token exists nowhere any more; losing it means regenerating it (and reconfiguring the phone).
- Importing devices from another system must compute the fingerprint from the existing token (`encode(sha256(convert_to(token::text,'UTF8')),'hex')`).
- Later work adds the WebSocket endpoint, asynchronous messaging and the real channels (SMTP, Free Mobile) on these ports.
