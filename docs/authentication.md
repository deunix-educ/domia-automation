# Authentication

**Server-side** sessions (opaque cookie, data in PostgreSQL), **argon2id** passwords, two roles: `admin` and `user`.
Decisions: [ADR 0004](adr/0004-auth-sessions.md) (in French).

## Creating and managing accounts

```bash
dominet createsuperuser --username admin [--email me@example.org]   # masked prompt, asked twice
dominet passwd --username admin                                     # also closes every session of the account
DOMINET_PASSWORD='...' dominet createsuperuser --username admin    # no prompt (scripts, Docker)
```

Password policy: 10 to 128 characters, not digits only, different from the username. Usernames: letters, digits and
`@ . + - _`; uniqueness is case-insensitive.

## Routes

| Route | Purpose |
|---|---|
| `POST /api/auth/login` `{"username","password"}` | opens a session, returns the account |
| `POST /api/auth/logout` | closes the session |
| `GET /api/auth/me` | signed-in account (401 otherwise) |
| `POST /api/auth/password` `{"current_password","new_password"}` | changes your password; **closes every session** |
| `GET /api/users` | list accounts (admin) |

Errors look like `{"error":{"code","message"}}`; `Retry-After` accompanies a 429.

## Security

- **Cookie** `dominet_session`: `HttpOnly`, `SameSite=Lax`, `Secure` (unless `COOKIE_SECURE=false`, plain HTTP on the LAN).
  Expires after `SESSION_IDLE_MINUTES` of inactivity (480) and at most `SESSION_MAX_DAYS` days (14) overall. The identifier is
  **renewed at login** (session-fixation protection).
- **Brute-force lockout** (15-minute window and duration): 5 failures for one account from one IP, or 20 for an account from
  anywhere, or 30 from one IP. While locked, the password is not even checked (even if correct). A successful login resets the
  account counters. Attempts are kept for 7 days.
- **No account enumeration**: unknown name, wrong password, disabled account and lockout look alike; a dummy computation of the same
  cost evens out response times.
- **CSRF**: any POST/PUT/PATCH/DELETE must come from the site's origin (`Origin` or `Referer` compared to `Host`) or from an origin
  listed in `CSRF_TRUSTED_ORIGINS`; `Sec-Fetch-Site: cross-site` is refused. Non-browser clients (curl, the Android app) are not affected.
- **Client IP** behind nginx: `X-Forwarded-For` is only trusted when the peer is in `TRUSTED_PROXIES`.
- **Headers**: `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy`, `Permissions-Policy`, `HSTS` over TLS,
  `Cache-Control: no-store` on `/api`. The `Content-Security-Policy` arrives with the Leptos front end (lots 7-8).
- Passwords are never logged nor printed (`Debug` of requests and configuration is masked).
