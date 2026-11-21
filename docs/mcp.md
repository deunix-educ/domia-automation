# MCP server (R11, in progress)

DomiNet's **MCP server** lets an assistant (Claude Code, Claude Desktop, or any MCP client) read, drive and configure the application under the same rules as the interface, **without ever seeing a secret**. Roadmap: `ROADMAP-mcp.md`; decisions: [ADR 0058](adr/0058-mcp-server.md) and [ADR 0059](adr/0059-mcp-access-tokens.md) (in French); audit: [R11-audit](mcp-audit.md).

!!! warning "State: work package M4d (configuration)"
    Tokens and audit (M1), read (M2), control and cameras (M3) and **configuration** (M4 to M4e, sixteen kinds) and **administration** (M5) are in place. Still to come: acceptance (M6).

## Enable
In `.env` (the server is **off by default**):
```bash
MCP_ENABLED=true
```
Then recreate the `dominet` service (`make prod-up`). The endpoint is `https://dominet.local/mcp`; it is reachable from the LAN and the VPN only. The `Host` header must be `localhost` or listed in `ALLOWED_HOSTS` (protection against DNS rebinding). **Without a token, nobody gets in.**

## Create a token
Page **Administration › Assistants (MCP)** (`/admin/mcp`): a name, the **scopes** and a validity period (30 to 365 days: a token without an end date does not exist). The token value, `dnm_…`, is **shown once**, with the command ready to paste into Claude Code; only its hash is kept, it cannot be read back. A lost token is revoked and created again.

| Scope | Allows |
|---|---|
| `lecture` (read) | look at devices, history, logs, alerts and configuration (secrets masked) |
| `pilotage` (control) | command devices, run scenes, acknowledge alerts |
| `configuration` | change automations, rules, dashboards and sources |
| `administration` | settings, cameras and Rhai scripts |

Scopes are **independent**: `configuration` does not grant `lecture`. Use a `lecture` token for everyday diagnosis and a wider one only for the time of a change.

## Connect Claude Code
```bash
claude mcp add --transport http dominet https://dominet.local/mcp --header "Authorization: Bearer <token>"
```
The workstation must trust DomiNet's certificate authority (`/dominet-ca.crt`). The command was verified with Claude Code on the development stack (address `http://127.0.0.1:8080/mcp`); the `https://dominet.local` trial on the EliteDesk is still to do (M6). To connect the assistant for one folder only, add `--scope local`; `claude mcp list` must show `dominet … ✔ Connected`, `claude mcp remove dominet` removes it.

## Step-by-step trial (development stack)
A complete trial that never touches production: the development stack is disposable.

1. **Start the stack and the server, MCP on**:
   ```bash
   make dev-up
   cargo leptos build                      # the first time, or after a change
   MCP_ENABLED=true tools/dev-run.sh       # server on http://127.0.0.1:8080, log line "serveur MCP monte sur /mcp"
   ```
2. **Create a token**: sign in at `http://127.0.0.1:8080` (development account and password in `deploy/.env.dev-run`), page **Administration › Assistants (MCP)**, name `trial`, the four scopes, 30 days. Copy the `dnm_…` value (shown once).
   Without the interface, for a trial only: insert into the development database the SHA-256 hash of a token of your choice (`mcp_token`, columns `name`, `token_hash`, `prefix`, `scopes`, `expires_at`).
3. **Connect Claude Code**, in a trial folder:
   ```bash
   mkdir -p ~/essai-mcp && cd ~/essai-mcp
   claude mcp add --transport http --scope local dominet http://127.0.0.1:8080/mcp \
     --header "Authorization: Bearer dnm_…"
   claude mcp list                         # dominet … ✔ Connected
   ```
4. **Ask questions** (interactive `claude`, or `claude -p "…" --allowedTools "mcp__dominet"`):
   - *Read*: "Give me the overall system status and which devices have been silent for 24 hours." (`system_status`, `devices_list`).
   - *Simulation*: "Propose, without writing anything, creating a place called 'Essai MCP'." (`config_schema`, `config_upsert` with `dry_run`).
   - *Write*: "Create the place, a 'Lampe-essai' device of type light attached to it, then an automation that sends a notification when a message arrives on `essai/bouton`." Then "Read the automation back with config_get".
   - *Secrets*: "Create a camera 'Cam essai' with the password `…`, then read it back." The password cannot be read back: only `has_rtsp_pass: true`.
   - *Safeguards*: "Delete the place without confirming." The call is refused (`confirm = true` required); `dry_run` says first what depends on the item.
5. **Check**: the **Administration › Assistants (MCP)** page shows the audit trail (one line per write or refusal, never a token value or a secret). The created items show up in the interface.
6. **Clean up**: `claude mcp remove dominet`, revoke the token on the MCP page, stop the server (Ctrl+C), `make dev-reset` to start again from an empty database.

**Result of the 2026-11-16 trial** (Claude Code, development stack): the assistant read the system status, simulated then really created a place, a device, an automation and a camera, read the automation back, noted it could not see the camera password, and was refused a deletion without `confirm`. The audit trail holds the four writes and no secret. Claude Code prints `unknown format "uint32" ignored in schema` warnings: harmless, they come from the output schemas.

## Claude Desktop (to be verified)
Claude Desktop cannot send an `Authorization` header to a remote server declared in its interface: use the `mcp-remote` bridge (Node.js required), in `claude_desktop_config.json`:
```json
{ "mcpServers": { "dominet": {
    "command": "npx",
    "args": ["-y", "mcp-remote", "https://dominet.local/mcp", "--header", "Authorization:${DOMINET_AUTH}"],
    "env": { "DOMINET_AUTH": "Bearer dnm_…", "NODE_EXTRA_CA_CERTS": "/path/dominet-ca.crt" }
} } }
```
This setup has **not** been tried here (no Claude Desktop on the development machine); it follows the usual `mcp-remote` method. A narrower token (`lecture` only) is the right choice for a mainstream client.

## Ready-made prompts
Three templates, offered with the `lecture` scope (they are plain text: they grant no right the token does not have). In Claude Code they are called `/mcp__dominet__diagnostic`, `/mcp__dominet__nouvelle_automation` and `/mcp__dominet__bilan_surveillance`.

| Prompt | Argument | What it makes the assistant do |
|---|---|---|
| `diagnostic` | `hours` (12) | system status, silent devices, alerts, recent errors, failing automations, then a 5-line summary |
| `nouvelle_automation` | `need` (required) | read the real devices and scenes, read the schema, propose in plain words, simulate (`dry_run`), create **only after your agreement** |
| `bilan_surveillance` | `hours` (24) | cameras, detections, recordings, energy, alerts; no snapshot or camera move unless asked |

## Check a server read-only
```bash
MCP_TOKEN=dnm_… tools/check-mcp.sh http://127.0.0.1:8080/mcp                   # development stack
CACERT=dominet-ca.crt MCP_TOKEN=dnm_… tools/check-mcp.sh https://dominet.local/mcp   # EliteDesk
```
The script checks the refusal without a token (401), counts the tools and prompts offered to the token and calls `system_status`. It writes nothing and never prints the token.

## Acceptance on the EliteDesk (for you to run)
Nothing is run on production without you. In order:

1. **Enable**: add `MCP_ENABLED=true` to the EliteDesk's `.env`, then `make prod-up` (the MCP server is off by default).
2. **Check the refusal**: `curl -s -o /dev/null -w '%{http_code}\n' -X POST --cacert dominet-ca.crt https://dominet.local/mcp` must answer **401**.
3. **Create a `lecture` token**, 30 days, page **Administration › Assistants (MCP)**.
4. **Check**: `CACERT=dominet-ca.crt MCP_TOKEN=dnm_… tools/check-mcp.sh https://dominet.local/mcp`.
5. **Connect Claude Code** (trusted workstation: `NODE_EXTRA_CA_CERTS=/path/dominet-ca.crt` if the CA is not installed) then `claude mcp add --transport http dominet https://dominet.local/mcp --header "Authorization: Bearer dnm_…"`.
6. **Diagnostic**: run `/mcp__dominet__diagnostic`. Compare with the home page and the Logs page.
7. **One reversible action** (a `pilotage` token, created for the occasion): ask to switch a lamp on then off; check the two audit-trail lines.
8. **Revoke** the trial tokens; keep `MCP_ENABLED=true` or remove it, as you prefer.

Note what differs from the development trial: certificate, host name (`ALLOWED_HOSTS`), real devices.

## Available tools (read)
Twenty tools, all at `lecture` (read) scope; an assistant only sees those its token allows.

| Tool | Purpose |
|---|---|
| `system_status` | version, database, active and unreachable devices, alerts firing, Zigbee pairing, machine load |
| `devices_list` | devices: reachable, last message, **silence in hours**, place, group; filters (`silent_for_hours`, reachable, type, name, inactive) |
| `device_get` | one device (name or number): current state, MQTT topics, recorded keys |
| `device_history` | history of a key (temperature, power…): points, minimum, maximum, average |
| `automations_list` | automations: trigger, action, last firing, firing count |
| `automation_get` | one automation in detail, **without a script's code** |
| `automation_runs` | run journal: when, outcome (`ok`, `error`, `skipped`, `limited`) and message; filter by automation and outcome |
| `scenes_list` | scenes and what each sends to each device |
| `alerts_list` | alerts (firing, acknowledged, resolved), severity, triggering value |
| `cameras_list` | cameras: active, priority, address, PTZ, ONVIF, last seen; **no credential** |
| `logs_query` | the application log since it started: level, module, text, window |
| `snapshots_list` | picture captures: number, camera, date, size; filter by camera and day |
| `snapshot_get` | **the image** of a capture (JPEG, 2 MB at most) |
| `recordings_list` | video recordings: camera, start, length, trigger, state |
| `detections_list` | motion and object detections (person, car…) |
| `energy_summary` | daily energy balance: Linky (peak and off-peak), solar, heat pump, with totals |

Resource `dominet://context`: the configuration snapshot (what `dominet dump-context` writes) as YAML, **without secrets and without script code** (replaced by a marker). The best starting point to understand an installation.

Questions that work: "Which devices have said nothing for 24 hours?" (`devices_list` with `silent_for_hours`: devices that never spoke are included, first), "Why did the *Ouvrir le volet* automation fail?" (`automation_runs` with `error`: the message gives the cause), "What has the office temperature been since this morning?" (`device_history`).

**Output rules**: **local** times, never UTC; no secret; a Rhai script's code is only readable at `administration` scope; text that comes from outside (names, messages, log lines) is marked as data (`notice`), never as instructions. Coming: control (M3), configuration (M4), settings and scripts (M5).

## Control tools (`pilotage` scope)
Nine tools act on the house. **Each accepts `dry_run = true`**: it then says what it would do, changing nothing and writing nothing to the audit trail.

| Tool | Purpose |
|---|---|
| `device_command` | sends a JSON command to an **active** device (`{"state": "OPEN"}`, `{"position": 40}`…); a deactivated device is not drivable |
| `scene_run` | runs an active scene: its commands go out in order, failures are reported |
| `alert_ack` | acknowledges one firing alert (`alert`) or all (`all`); the acknowledgement is attributed to the **account that created the token** |
| `notify_broadcast` | shows a banner (200 characters) to every signed-in account |
| `notify_test` | sends a test e-mail to the alert recipient (address never shown); **SMS do not go through MCP** |
| `camera_ptz` | moves a PTZ camera for 100 to 3,000 ms (`up`, `down`, `left`, `right`, `zoom_in`, `zoom_out`) then **stops it by itself**; the stop is guaranteed even if the connection drops |
| `camera_night` | switches day / night, infrared filter or light, through the method set for the camera |
| `camera_record` | starts or stops recording on a camera set to **manual** mode (otherwise refused with a reason) |
| `camera_snapshot` | takes a picture now (the video role must be running) and returns its number |

**No action without a trace.** Every call that changes something writes its line to the audit trail **before** it runs (outcome `pending`), then closes it with what happened (`ok` or `error`). If the trail cannot be written, **the call is refused and nothing is sent**. A call refused for lack of scope, an inactive device or an invalid command leaves no action: only the scope refusal is recorded (`denied`). Decisions taken on the **built-in assistant** (R12) have their own type, `agent`: the request in clear, the action confirmed or turned down, and what it takes to undo it (`ROADMAP-agent.md` section 14).

## Configuration (`lecture` and `configuration` scopes)
Seven tools read and write the configuration, for sixteen kinds: **place** (`location`), **scene** (`scene`), **automation** (`automation`), **alert rule** (`alert_rule`), **device group** (`device_group`), **state-key label** (`device_label`), **MQTT source** (`mqtt_source`) **weather station** (`weather_station`), **Prometheus rule** (`prometheus_rule`) **Grafana rule** (`grafana_rule`), **dashboard** (`dashboard`, named `owner/name`) **widget** (`widget`, placed on the 12-column grid), **device** (`device`: its parameters, never its state), **navigation-bar entry** (`navbar`, Lucide icon checked) and **camera** (`camera`, `administration` scope) and the object **detector** (`detector`: a single item, changed but never created or deleted). A widget that would leave the grid or overlap another is refused at `dry_run`; its text content is sanitised; deleting a dashboard deletes its widgets (the `dry_run` gives the count); deleting a device deletes its history and leaves its widgets without a device. Saving or deleting a rule rewrites `rules.yml` or `rules.yaml` and reloads the service (best effort: a failed reload does not undo the save, the rules page shows the state).

| Tool | Scope | Purpose |
|---|---|---|
| `config_kinds` | read | the configurable kinds, with their read and write scopes |
| `config_schema` | read | what `config_upsert` accepts for a kind: JSON schema, **full example** and notes (the keys of each trigger and action) |
| `config_list`, `config_get` | read | the items of a kind, **in the exact shape `config_upsert` accepts**: read, change a field, write |
| `config_upsert` | configuration | creates (no `id`) or **replaces** (with `id`) an item; `dry_run` shows the fields that would change |
| `config_delete` | configuration | deletes; **`confirm = true` required**, `dry_run` says what depends on it |
| `zigbee_pairing` | configuration | opens (254 s at most) or closes Zigbee pairing |

**The same checks as the interface**, because the path is the same: a shared service (`dominet_server::config_service`) validates the domain draft, checks that a **Rhai script compiles** and that the **target scene exists**, turns name conflicts into field errors, then reloads the automation engine, the scheduler or the alert evaluation. A `dry_run` therefore announces exactly the refusals of a real write. Unknown fields are refused (a typo is not silently ignored).

**Rhai code** (an automation's script, an MQTT source's `handler_code`) and **a Grafana rule's SQL query** (any datasource other than `prometheus`): never readable (`script` is `null`, `has_script` says so). Writing, changing or removing it needs the `administration` scope **in addition**. An update that **does not mention** `script` **keeps** the existing code: reading an automation, changing one field and writing it back never erases its code.

Example, end to end: "close the shutters at sunset" → `config_schema` (kind `automation`) → `config_upsert` with `dry_run`, then without. The `dominet://context` snapshot before and after differs by that automation only (it is a test).

## Administration (`administration` scope)

The `administration` scope opens what is sensitive: settings, script code, cameras. It is independent of the others and rarely given.

- **Settings**: `settings_list` (the value of a secret is **never** returned, only `is_set`) and `setting_set` (value validated against the setting's type; a secret is encrypted; `dry_run` says whether the value would be accepted).
- **Rhai scripts**: `script_get` reads the code of an automation (`script` action) or of an MQTT source; `script_check` compiles it and, given a `topic`, tries it on a sample message in the real sandbox, publishing nothing; `script_set` replaces it after the same checks as the interface. No other tool ever shows that code.
- **Cameras**: the `camera` kind of `config_*` (reading, writing and deleting all need `administration`, since it holds addresses and user names). The passwords (`rtsp_pass`, `onvif_pass`, `web_pass`) and the API key (`web_api_key`) are **write-only**: `config_get` only says `has_rtsp_pass`…; a field left out keeps the secret in place. `cameras_sync` updates MediaMTX (`target = mediamtx`) or learns what a camera announces (`target = onvif`).

No secret appears in any answer or in the audit trail: a test sweeps the outputs.

## Security
- **Immediate revocation**: a revoked or expired token is refused from the next call.
- **Rate limit** per token (`MCP_RATE_LIMIT_PER_MIN`, 120 by default): beyond it, **429** with `Retry-After`.
- **Audit trail** (same page): token creation and revocation, refused calls and, from work package M3, calls that change something. Secrets are masked there; purged after `MCP_AUDIT_RETENTION_DAYS` (365 days by default).
- Without a token, or with a wrong, expired or revoked one, the answer is **401** (`WWW-Authenticate: Bearer`); with an unknown `Host`, **403**; server disabled, **404**. Neither the `Authorization` header nor a token value is logged.

## Tests
`cargo test -p dominet-mcp`: a real MCP client (`rmcp`) against a real server and the test database, **every read tool on real data** (`--test read`), token read from the database, revocation, expiry, refused and audited scope, rate limit, `Host` protection, a client on the previous protocol revision, disabled server; `--test coverage`: every server function of the interface must be classified in `tests/mcp/coverage.tsv`. Repositories: `cargo test -p dominet-store --test mcp`.
