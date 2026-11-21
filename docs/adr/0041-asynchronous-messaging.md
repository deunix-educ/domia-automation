# 0041 — Messaging: asynchronous sending, live statuses, rights

- Status: accepted
- Date: 2026-11-07

## Context
Internal messaging (one message, several accounts, e-mail and/or SMS) exists in the reference with **synchronous** sending: the request waits for each e-mail, and an SMS stays "pending" without a maximum delay. The channels (ADR 0038), the Android relay (ADR 0039) and the data model (ADR 0037) exist.

## Options
- **Synchronous, like the reference**: simple, but the page blocks while the SMTP answers (up to 10 s per recipient) and nothing is visible before the end.
- **Asynchronous with a database queue and a worker**: robust to restarts, but heavy for a few messages a month.
- **Asynchronous in a background task** (chosen): the message and its recipients are saved in one transaction, then a `tokio` task does the sends.

## Decision
- **Sending (`dominet_server::messaging::send`)**: creates the message (`sending`) and one recipient per account with `initial_statuses` (`pending` only if the channel is requested **and** the account has the address / mobile, otherwise `na` — a deliberate deviation from the reference, which marked "failed: no address"), then hands back control. The detail page opens at once.
- **Background task**: one send at a time for the whole server (global lock); for each recipient, the e-mail goes through `Channels::send_email` (subject = the message's subject, plain text), then the SMS is handed to the **most recently seen online** Android device, of any owner (like the reference). Without a device: `sms: no Android device connected`; device dropped meanwhile: `sms: device unreachable`. An e-mail failure is written `email: <reason>`.
- **SMS**: stays `pending` until the application's acknowledgement, or up to 60 s (ADR 0039, `expire_stale`). Settlement goes through `android::settled`, which updates the recipient and closes the message.
- **End**: the message becomes `sent` (with `sent_at`) as soon as nothing is pending (`finish_if_complete`, atomic). `sent` means "processed", not "all succeeded": the table says what failed.
- **Audience**: `all` (active accounts), `staff` (active administrators), plus chosen accounts; cumulative without duplicates (`resolve_audience`). An inactive account chosen explicitly is served (like the reference) but the form only offers active ones.
- **Live**: topic `message:<id>` of the WebSocket (`WsServerMsg::MessageProgress { message_id, complete }`), published at every processed recipient and every acknowledgement. Allowed to the sender, recipients and administrators; an unknown message and another's message get the same refusal. The browser rereads the page at each event (like the Android pages, ADR 0040). Events are not kept by the hub: the page rereads anyway on opening.
- **Rights**: compose and list sent messages = administrators; inbox = any account; detail = sender, recipient or administrator — the sender and administrators see the full table, a recipient only their own state. Unknown or foreign message: "not found".
- **Resume at start-up**: a message left `sending` after a hard stop has its pending sends declared failed ("interrupted by a server restart") then is closed (`recover_interrupted`), so nothing stays "in progress" indefinitely. It is not known whether the interrupted send went out: failure is the prudent answer, the sender may resend.
- **Pages**: `/messaging` (sent), `/messaging/inbox`, `/messaging/compose`, `/messaging/<id>`; the last 100 messages per list.

## Verification
- `crates/dominet/tests/messaging.rs` (6 tests, a fake SMTP and a phone that talks like the application): e-mails received with sender, subject and text; SMS handed over with the right number; statuses before and after the acknowledgement; an SMTP that refuses and no phone; an SMS without acknowledgement failed after 60 s; events up to `complete`; subscription rights (sender, recipient, administrator, foreign, unknown); resume after restart.
- Unit: audience composition.
- **End-to-end `tests/e2e/messaging.py`** (31 checks): form refused empty, send, live status tracking with a hand-made acknowledgement, simulated SMTP (`tools/sim-smtp.py`), the recipient's inbox without the others' table, reserved pages, failure without a phone with its reason.
- No real send: tests never reach a real SMTP host or Free Mobile.

## Consequences
- A server that stops during a send may have sent an e-mail that the database declares failed; resending is safe for the server, but the recipient could receive it twice.
- Sending is globally serialised: enough for a few recipients, to revisit (bounded queue, limited concurrency) if volumes changed.
- No drafts or attachments (the schema's `draft` state stays unused).
