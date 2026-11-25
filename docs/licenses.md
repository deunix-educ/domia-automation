# Licences and third-party components

DomiNet is distributed under the **GNU Affero General Public License, version 3 or later** (`LICENSE`, `SPDX-License-Identifier: AGPL-3.0-or-later` in every source file). If you run a modified version for other people over a network, the AGPL requires you to offer them the corresponding source.

This page lists what the project embeds, builds with, ships in its image, or talks to, with the licence of each and **how far that was checked**. Checked on 2026-11-21. Licences change: re-check before a release.

Status: **verified** = read at the upstream source on that date; **reported** = found through a search, not read at the source; **known** = the project's long-standing published licence, not re-read.

## In the source tree

| Component | Licence | Where | Status |
|---|---|---|---|
| Rust crates (about 590 external) | MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, MPL-2.0 (4), CC0, others permissive; some dual-licensed with LGPL as an option only | `Cargo.lock` | **verified** by `cargo deny check licenses` (part of `make ci`), allow-list in `deny.toml` |
| Lucide icons v1.21.0 | ISC (portions MIT, from Feather) | `crates/dominet-server/data/lucide.tsv`, notice in `LUCIDE-LICENSE.txt` | **verified** (notice present; kept with the data) |
| SolarMax protocol (request and answer format, quantities) | **GPL-3.0-or-later**, "Developed 2009-2010 by Bernd Wurst", published 2012 (`python-solarmax`) | `crates/dominet-integrations/src/solarmax/protocol.rs` and its frozen test vectors | **verified** at `git.schokokeks.org/python-solarmax.git` |
| Test reference files (`tests/fixtures`) | derived from the SolarMax protocol above, from the project's own data and from Open-Meteo answers | `tests/fixtures/` | derivation known |

GPL-3.0-or-later code can be included in an AGPL-3.0-or-later work (section 13 of both licences allows the combination). The SolarMax notice previously said "GPL-2.0-or-later": that was wrong and has been corrected.

## Built or downloaded, not committed

| Component | Licence | Notes | Status |
|---|---|---|---|
| YOLOv8n weights, exported by `make fetch-model` (Ultralytics) | **AGPL-3.0** | Not in the repository (ignored by git) but copied into the Docker image. Consistent with the project's own AGPL; anyone who wants to use the model in a non-AGPL product needs an Ultralytics commercial licence | **known** |
| ONNX Runtime (prebuilt binary downloaded at build by the `ort` crate) | MIT | Copied next to the binary | **known** |
| `ffmpeg` (Debian package in the image) | LGPL or GPL, depending on the Debian build | Separate program started by the application; Debian provides its source | **known** |
| Intel media driver (amd64 image only) | MIT | | **known** |
| MkDocs 1.6.1, Material 9.8.1, static-i18n 1.3.1 | BSD-2-Clause, MIT, MIT | Documentation build only; the generated site is served by the application. **Material for MkDocs is in maintenance until 2027-05-05; its successor is Zensical** | **known** |

## Services started by Docker Compose (pulled, not redistributed by this project)

| Image | Licence | Status |
|---|---|---|
| `postgres:16-alpine` | PostgreSQL licence | known |
| `reduct/store:v1.21.5` | Apache-2.0 (main branch; the pinned tag was not read) | **verified** for `main` |
| `eclipse-mosquitto:2.0` | EPL-2.0 / EDL-1.0 | known |
| `bluenviron/mediamtx:1.21.3` | MIT | **verified** |
| `prom/prometheus:v2.52.0` | Apache-2.0 | known |
| `grafana/grafana:11.0.0` | AGPL-3.0 | known |
| `nginx:1.27-alpine` | BSD-2-Clause | known |
| `koenkk/zigbee2mqtt:2.1.1` (optional) | GPL-3.0 | **verified** |
| `adminer:4.8.1` (optional) | Apache-2.0 or GPL-2.0 | known |
| `ghcr.io/wg-easy/wg-easy:15` (optional VPN) | AGPL-3.0 (version 14 and earlier were **CC BY-NC-SA 4.0, non-commercial**) | **verified** at the source on 2026-11-23: the `LICENSE` file at tag v14.0.0 is CC BY-NC-SA 4.0, at tag v15.0.0 and on `master` it is the AGPL-3.0 text |

**VPN profile.** It uses wg-easy 15 (AGPL-3.0, like DomiNet) since [ADR 0065](adr/0065-wg-easy-15.md); version 14, which was non-commercial only, is no longer used or recommended.

## External services

| Service | Terms | What it means for you |
|---|---|---|
| **Open-Meteo** (weather; `OPEN_METEO_URL`) | The free API is for **non-commercial use** (limits: 10 000 calls a day, 5 000 an hour, 600 a minute); the data is under **CC BY 4.0, which requires attribution** | **reported** (terms page seen through a search). A home installation stays far below the limits. The weather widget shows the attribution **"Weather data by Open-Meteo.com"** (linked to open-meteo.com); keep it wherever you republish the data. A commercial use needs a paid plan, or another provider set in `OPEN_METEO_URL` |
| Free Mobile SMS API | the operator's terms, for your own line | known |
| SMTP server, Prometheus/Grafana alert e-mails | yours | |

## When you redistribute

- Keep `LICENSE`, `LUCIDE-LICENSE.txt` and the SolarMax notice in `protocol.rs`.
- A published Docker image contains AGPL (the application, the model), GPL/LGPL (`ffmpeg`) and permissive components: publish the corresponding source, which is this repository at the tag the image was built from.
- Do not remove the licence headers; add `SPDX-License-Identifier: AGPL-3.0-or-later` to new files.
