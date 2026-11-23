# To improve — interface

List kept by the user and by Claude; a ticked line is done.

## Page presentation (asked on 2026-11-07)
- [x] **Line spacing and margins** (CSS pass of 2026-11-08): card paragraphs, line height, `vc-list` lists. Still to be reviewed in the user's browser, page by page (dashboard, devices, energy, Android).
- [x] **Fields that overlap or misalign**: generic `crud` forms on two columns (screen ≥ 960 px), checkboxes with their help text below, framed choice groups; messaging and Android forms moved to `vc-form-group`. `/admin/cameras` fields are grouped in pairs.
- [x] **"Write a message" page** (`/messaging/compose`): stacked fields, channel and recipients in frames.
- [x] Discovery page `/surveillance/discovery`: cards, one field per line (R6 lot 3).

Tool: `SHOT_DIR=… tools/e2e.sh shots` takes screenshots of several pages (list set with `SHOT_PAGES`, e.g. `admin/cameras!add`).

Proposed method: a single CSS pass (`crates/dominet-web/style/main.css`) then review in the user's browser, optimised build (`RELEASE=1 make dev-run`). One page at a time, before/after screenshots if useful.

## Other
- The development WASM (48 MB) makes the interface slow to react: use `RELEASE=1 make dev-run`, or reduce the development WASM profile (opt-level, `wasm-opt`).

## Language follow-ups (English-first repository)
- **Local assistant is French-only**: its system prompt, the phrases it detects ("do you want me to continue?"), the action-card texts and the tool-result markers are in French on purpose (the assistant answers in French). Generalising it means translating them, answering in the user's language and replaying the evaluation set (`make agent-eval`).
- **MCP scope identifiers are French** (`lecture`, `pilotage`; `configuration` and `administration` are shared): they are stored in token rows. Renaming them to `read` / `control` needs a data migration with accepted aliases.
- **Notification texts** (alert e-mails and SMS, device messages) are English now; a per-account language is not implemented.
- **Code comments** are mostly French (several thousand lines): they are translated gradually; new code is commented in English.
- **Example data in configuration schemas** (names such as "Fermeture du soir") and the French aliases accepted for configuration kinds (`regle_d_alerte`…) stay as they help a French-speaking model.

## Licences follow-ups
- **wg-easy**: version 14 (pinned for the optional VPN) is non-commercial (CC BY-NC-SA 4.0) and version 15 is AGPL-3.0, both verified at the source on 2026-11-23; version 15 is a rewrite. Move to 15 (configuration, `wgpw` command in `DEPLOY.md`) and test the VPN profile. See [Licences](licenses.md).
- **Re-read the pinned tags** (`reduct/store:v1.21.5` and the other images marked "known") before the first public release.
- **Camera viewing without a password on the LAN** (RTSP 8554, WebRTC 8189): add per-viewer credentials in MediaMTX and in the application. See [Security](security.md).

