# Notifications: sending channels

Since R5 (lot 2, [ADR 0038](adr/0038-delivery-channels.md)), DomiNet can send **e-mails** (SMTP) and **Free Mobile SMS**. The `dominet-notify` crate knows neither the database nor the UI; `dominet_server::notify::Channels` assembles it with the configuration.

## What is configured where

| What | Where | Key |
|---|---|---|
| SMTP server, port, credentials, encryption | environment (restart) | `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS`, `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS` |
| Sender | setting (immediate) | `DEFAULT_FROM_EMAIL` |
| Alert recipient | setting | `EMAIL_RECIPIENT` (falls back to the sender) |
| Free Mobile credentials | settings (the key is encrypted at rest) | `SMS_FREE_USER`, `SMS_FREE_KEY` |
| Free Mobile API address | environment | `SMS_FREE_URL` (default: the official API) |

## Security

- `EMAIL_USE_TLS` (default): `STARTTLS` is **mandatory**, never a plaintext fallback; `EMAIL_USE_SSL`: TLS from the start (port 465). Both together are rejected. Without encryption, credentials are only allowed towards the loopback (otherwise the configuration is refused at startup).
- The SMTP password and the Free Mobile key appear neither in `Debug`, nor in logs, nor in errors (the key travels in the HTTP address: `reqwest` errors are written without the address, and redirects are refused).
- A line break in a subject is replaced by a space: no header injection.
- Free Mobile's HTTP status is checked (400, 402, 403, 500 explained). The message is cut at 160 characters, 5 s timeout.

## Trying it for real: `dominet notify-test`

```
dominet notify-test --email            # to the alert recipient
dominet notify-test --email --to me@example.com
dominet notify-test --sms
```

**This is the only command that really sends** (besides alerts, automation notifications and messaging, which also really send as soon as SMTP is configured): it only runs when you ask. Tests, the demo and development talk to fake servers (`dominet-notify`, `testing` feature: `FakeSmtp`, `FakeFreeMobile`) and never leave the machine.
