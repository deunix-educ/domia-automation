# 0038 — Delivery channels: e-mail (SMTP) and Free Mobile SMS

- Status: accepted
- Date: 2026-11-06

## Context
The reference sent e-mails through a framework mail module (Gmail SMTP) and alert SMS through an HTTP GET on the Free Mobile API, without checking the return code. Both channels are taken over for alerts, messaging and the `notification` automation action, without ever sending for real during development.

## Options
- **E-mail**: `lettre` (asynchronous SMTP, `rustls`); or a `sendmail` command; or an HTTP call to a third-party service. `lettre` takes over the usual variables and adds no service.
- **Tests**: a third-party simulation crate, or small home-made fake servers on loopback. A home-made fake server records exactly what is received and also serves the demonstration.
- **What is set where**: everything in the environment, or everything in settings. The SMTP server (host, password) is an installation matter; the sender, the recipient and the Free Mobile credentials already exist as settings modifiable without a restart.

## Decision
- **`dominet-notify` crate** (no database or interface): `SmtpSender` (`lettre`, plain UTF-8 text, one connection per message, configurable timeout) and `FreeMobile` (`reqwest`, GET `?user&pass&msg`, 160 characters, 5 s). The server adaptations live in `dominet_server::notify::Channels`.
- **Environment**: `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS` (default true), `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS` (10), `SMS_FREE_URL`. **Settings** (already in the registry): `DEFAULT_FROM_EMAIL`, `EMAIL_RECIPIENT` (the sender by default), `SMS_FREE_USER`, `SMS_FREE_KEY` (encrypted secret).
- **Encryption without fallback**: `STARTTLS` is mandatory (a connection that does not obtain it fails, nothing is sent in clear); direct TLS with `EMAIL_USE_SSL`; `TLS` and `SSL` together are a configuration error. **Credentials without TLS are only accepted towards loopback**: the configuration is refused at start-up otherwise.
- **No secret in output**: `Debug` masks the password and the key; HTTP errors are written without the address (`without_url`, since the key is in the address); redirects are refused so the key does not follow.
- **Deliberate deviations from the reference**: Free Mobile's HTTP code is checked (400, 402, 403, 500 explained); a line break in a subject becomes a space.
- **Fake servers** (`dominet_notify::testing`, feature `testing`): `FakeSmtp` (EHLO, AUTH PLAIN / LOGIN, MAIL / RCPT / DATA, configurable refusals, no `STARTTLS`) and `FakeFreeMobile` (configurable HTTP code). No test, nor the demonstration, contacts a real SMTP server or Free Mobile.
- **`dominet notify-test --email [--to] --sms`**: a real send of a test message, the **only real path**, run by the user.

## Consequences
- `STARTTLS` / TLS encryption is not exercised by the tests (the fake server is in clear): it is by `notify-test` on the user's side; the tests only check that it is required.
- A change of `EMAIL_*` needs a restart; a settings change is picked up at the next send (no cache).
- Later work (alerts, messaging, the `notification` action) calls `Channels`; the `Notifier` port keeps its interface.
