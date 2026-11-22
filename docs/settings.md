# Application settings

Seventeen settings editable without a restart, stored in the database (`app_setting`), typed (`str`, `int`, `float`, `bool`)
and grouped by category. A setting missing from the database takes its **default value**: the application works before `dominet seed`.

| Category | Key | Type | Default |
|---|---|---|---|
| Notifications | `EMAIL_RECIPIENT` | str | `dominet@localhost` |
| Notifications | `DEFAULT_FROM_EMAIL` | str | `dominet@localhost` |
| Notifications | `SMS_FREE_USER` | str | *(empty)* |
| Notifications | `SMS_FREE_KEY` | str, **secret** | *(empty)* |
| Alerts | `PROMETHEUS_ALERT_COOLDOWN` | int (s) | `3600` |
| Alerts | `MOTION_ALERT_COOLDOWN` | int (s) | `600` |
| Surveillance | `MOTION_THRESHOLD` | float (0.0–1.0) | `0.01` |
| Surveillance | `RECORDINGS_SEGMENT_DURATION` | int (s) | `60` |
| Interface | `GRAFANA_PUBLIC_URL` | str | `http://localhost:3000` — origin allowed for the dashboard's Grafana iframes (CSP `frame-src`, ADR 0022) |
| Interface | `PROMETHEUS_PUBLIC_URL` | str | `http://localhost:9090` |
| Interface | `MEDIAMTX_WEBRTC_DOMAIN` | str | `localhost:8889` |
| Interface | `MEDIAMTX_WEBRTC_SCHEME` | str | `http` |
| MQTT | `Z2M_BASE_TOPIC` | str | `zigbee2mqtt` |
| Assistant | `AGENT_ENABLED` | bool | `false` — enables the Assistant page (R12) |
| Assistant | `AGENT_LLM_URL` | str | *(empty)* — address of `llama-server`, ex. `http://192.168.1.20:8081` |
| Assistant | `AGENT_MODEL` | str | `Qwen3-8B-Q4_K_M` — everyday requests |
| Assistant | `AGENT_MODEL_ADMIN` | str | *(empty)* — model for configuration requests; empty = same as `AGENT_MODEL` |

Values: decimals use a **dot** (`0.01`), booleans `true/false/oui/non/1/0`. A value incompatible with the type is refused (400).

## Secrets

A key containing `KEY`, `PASSWORD`, `TOKEN` or `SECRET` is a **secret**: encrypted in the database (AES-256-GCM, ADR 0011), never
returned by the API (`********`), never logged. It requires `DOMINET_MASTER_KEY`:

```bash
dominet gen-master-key          # prints a key: copy it into DOMINET_MASTER_KEY in the .env file
```

**Back this key up separately from the database**: without it, stored secrets are lost.

## Initial data

```bash
dominet seed [--dry-run]
```

- Creates missing settings. Initial value: the environment variable of the same name if set (handy in Docker, e.g. `SMS_FREE_KEY`
  in `.env`), otherwise the default. **Never overwrites** an existing setting; safe to re-run.
- Creates the default navigation (7 entries) **only when the table is empty**: an entry the administrator deleted or renamed is not recreated.
- Checks everything before writing: on error (secret without master key, invalid value), nothing is written.
- `--dry-run` shows what would be created without writing. Values are never printed.

## Routes (administrator)

| Route | Purpose |
|---|---|
| `GET /api/settings` | the 17 settings (display value, `is_secret`, `is_set`, `from_default`, `readable`) |
| `PUT /api/settings/{key}` `{"value": "..."}` | changes a setting; 400 if invalid, 404 if unknown, 503 for a secret without master key |

Every change updates the cache immediately and notifies subscribed components (live reconfiguration).
