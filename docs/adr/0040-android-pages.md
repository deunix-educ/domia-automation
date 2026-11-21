# 0040 — Android pages: rights, token shown once, live update

- Status: accepted
- Date: 2026-11-07

## Context
The relay (ADR 0039) receives SMS and calls; the pages that show them (`/android/…`), device registration and SMS sending from the browser are needed.

## Decision
- **Four pages**: dashboard `/android`, devices `/android/devices`, SMS `/android/sms?device=N`, call log `/android/calls?device=N`. Operations (add, delete, new token, send) are server functions, not confirmation pages: deletion asks for confirmation in place.
- **Rights**: any signed-in account adds its devices and sees its own; an administrator sees all devices (repositories take `Option<UserId>`, `None` = all). A device that is not the account's is "not found" (the same answer as an unknown identifier), for pages, sending, deletion and new token.
- **Token shown once**: only creation and regeneration return it (`NewAndroidDeviceDto`, whose `Debug` masks the token); no list, no reload page contains it. A banner shows it with the address to enter and a "hide" button.
- **Live**: pages follow the `android:<id>` topics of their devices; each event increments a counter (`Realtime::android`) that restarts the page's read. A simple choice: reread instead of merging events (100 rows at most, light queries), so the screen always equals the database.
- **Sending**: the device must be connected (otherwise refusal, nothing recorded); the outgoing SMS is recorded, put in the queue (`sms_outbox`) then handed to the device; other tabs receive `SmsQueued`. The page shows each outgoing SMS's state (**pending**, **sent**, **failed** with its reason) read from the queue; without acknowledgement after 60 s: failed.
- **Read at display**: opening the SMS page marks the device's received SMS as read. No purge, 100 rows per page.
- **Validation**: field by field (device name and number; recipient, message).

## Verification
- `tests/ui.rs`: rights per account, validation, token returned once (and absent from lists), read marking, deletion, regeneration.
- `android_ws.rs`: an SMS written in the browser reaches the application and its state follows the acknowledgement (sent, failed with reason).
- **End-to-end `tests/e2e/android.py`** with a **simulated phone** (`tools/sim_android.py`: a standard-library WebSocket client, same JSON frames as the Kotlin application, answering control frames like OkHttp): add from the interface, presence, SMS and calls live, sending with acknowledgement and failure, refusal when offline, new token (the old one is cut with 4001), deletion.

## Consequences
- The address to enter in the application is `ws://host/ws/android/device/<token>/`; `wss://` comes with TLS.
- A page rereads at every event: if volume became large, events would be merged in the browser.
