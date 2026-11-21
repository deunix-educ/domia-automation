# 0042 — Alerts and automation notifications on real channels

- Status: accepted
- Date: 2026-11-07

## Context
The `Notifier` port (ADR 0035) was laid down: a fired alert produces a banner, its three channels (e-mail, SMS, Android) are recorded but not served, `notified` stays false. The channels (ADR 0038) and the Android relay (ADR 0039) are now delivered.

## Decision
- **Alerts**: `AlertNotifier` (`dominet_server::alerts`) replaces `UiNotifier`. On `fired`: the banner goes at once, then a background task serves **the rule's** channels — e-mail to the alert recipient (`EMAIL_RECIPIENT`, the sender by default) with the subject `[DomiNet Alert <severity>] <rule>` and the long text; Free Mobile SMS with the short text (160 characters); an `alert_notification` push to **all** online Android devices. Alert evaluation never waits for a send.
- **Isolation**: each channel is tried even if another fails; an **unconfigured** channel (no SMTP, no Free Mobile credentials) is skipped with an information message, a real failure is logged (never the secret). `notified` becomes true after the attempt, failures or not; failures are not retried (the rule's repeat delay takes care of it).
- **Android push kept** although the current application ignores it ("Message ignored"): it is compatible with a future version, and the rules interface says so (a note under the checkbox).
- **Free Mobile HTTP code checked** (the reference ignored it): a refusal is a logged failure.
- **Automation notification**: the `notification` action keeps the banner and also sends **an e-mail** to the alert recipient (subject `[DomiNet] Notification`) and, **if the automation ticks "SMS"**, a Free Mobile SMS (`Channels::notify`). A channel that fails does not prevent the other.
- **No real send**: tests and the demonstration use the fake SMTP, the fake Free Mobile and a simulated phone.

## Before a first real use
Automations that have a `notification` action (and alerts whose e-mail / SMS boxes are ticked) will **really** send messages as soon as the server is connected to a real SMTP and real credentials: check with `dominet notify-test`, run by yourself.

## Verification
- `crates/dominet/tests/alert_channels.rs` (5 tests): the **real alert engine** fires, the e-mail (sender, recipient, subject, text), the SMS (credentials, short text) and the push received by a connected phone are exactly the expected ones; only the rule's channels serve; e-mail refused: the SMS goes and `notified` becomes true; an unconfigured channel skipped; automation notification (SMS only if requested, failure reported).
- End-to-end: the alert e-mail received by the simulated SMTP and the alert marked notified, on the Docker image.

## Consequences
- An alert whose send fails is not replayed: only the next firing (after the rule's delay) retries.
- `Channels` is in `AppState`: any future notification (Prometheus alerts) uses it as is.
