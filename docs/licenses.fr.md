# Licences et composants tiers

DomiNet est distribué sous la **GNU Affero General Public License, version 3 ou ultérieure** (`LICENSE`, `SPDX-License-Identifier: AGPL-3.0-or-later` dans chaque fichier source). Si vous exécutez une version modifiée pour d'autres personnes, par le réseau, l'AGPL vous oblige à leur proposer le code source correspondant.

Cette page liste ce que le projet embarque, utilise pour se construire, met dans son image ou interroge, avec la licence de chacun et **jusqu'où cela a été vérifié**. Vérifié le 2026-11-21. Les licences changent : revérifiez avant une version.

Statut : **vérifié** = lu à la source amont à cette date ; **rapporté** = trouvé par une recherche, pas lu à la source ; **connu** = la licence publiée de longue date du projet, non relue.

## Dans l'arborescence

| Composant | Licence | Où | Statut |
|---|---|---|---|
| Crates Rust (environ 590 externes) | MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, MPL-2.0 (4), CC0, autres permissives ; certaines en double licence avec LGPL en option seulement | `Cargo.lock` | **vérifié** par `cargo deny check licenses` (dans `make ci`), liste autorisée dans `deny.toml` |
| Icônes Lucide v1.21.0 | ISC (parties MIT, venant de Feather) | `crates/dominet-server/data/lucide.tsv`, avis dans `LUCIDE-LICENSE.txt` | **vérifié** (avis présent, conservé avec les données) |
| Protocole SolarMax (format des requêtes et réponses, grandeurs) | **GPL-3.0-or-later**, « Developed 2009-2010 by Bernd Wurst », publié en 2012 (`python-solarmax`) | `crates/dominet-integrations/src/solarmax/protocol.rs` et ses vecteurs de test figés | **vérifié** sur `git.schokokeks.org/python-solarmax.git` |
| Fichiers de référence des tests (`tests/fixtures`) | dérivés du protocole SolarMax ci-dessus, des données du projet et de réponses d'Open-Meteo | `tests/fixtures/` | dérivation connue |

Du code GPL-3.0-or-later peut être inclus dans une œuvre AGPL-3.0-or-later (l'article 13 des deux licences autorise la combinaison). L'avis SolarMax disait auparavant « GPL-2.0-or-later » : c'était faux, c'est corrigé.

## Construit ou téléchargé, pas commité

| Composant | Licence | Remarques | Statut |
|---|---|---|---|
| Poids YOLOv8n, exportés par `make fetch-model` (Ultralytics) | **AGPL-3.0** | Pas dans le dépôt (ignoré par git) mais copié dans l'image Docker. Cohérent avec l'AGPL du projet ; qui veut utiliser le modèle dans un produit non AGPL a besoin d'une licence commerciale Ultralytics | **connu** |
| ONNX Runtime (binaire précompilé téléchargé à la construction par la crate `ort`) | MIT | Copié à côté du binaire | **connu** |
| `ffmpeg` (paquet Debian de l'image) | LGPL ou GPL selon la construction Debian | Programme séparé lancé par l'application ; Debian fournit ses sources | **connu** |
| Pilote média Intel (image amd64 seulement) | MIT | | **connu** |
| MkDocs 1.6.1, Material 9.8.1, static-i18n 1.3.1 | BSD-2-Clause, MIT, MIT | Construction de la documentation seulement ; le site généré est servi par l'application. **Material for MkDocs est en maintenance jusqu'au 2027-05-05 ; son successeur est Zensical** | **connu** |

## Services lancés par Docker Compose (téléchargés, pas redistribués par ce projet)

| Image | Licence | Statut |
|---|---|---|
| `postgres:16-alpine` | licence PostgreSQL | connu |
| `reduct/store:v1.21.5` | Apache-2.0 (branche principale ; l'étiquette utilisée n'a pas été lue) | **vérifié** pour `main` |
| `eclipse-mosquitto:2.0` | EPL-2.0 / EDL-1.0 | connu |
| `bluenviron/mediamtx:1.21.3` | MIT | **vérifié** |
| `prom/prometheus:v2.52.0` | Apache-2.0 | connu |
| `grafana/grafana:11.0.0` | AGPL-3.0 | connu |
| `nginx:1.27-alpine` | BSD-2-Clause | connu |
| `koenkk/zigbee2mqtt:2.1.1` (facultatif) | GPL-3.0 | **vérifié** |
| `adminer:4.8.1` (facultatif) | Apache-2.0 ou GPL-2.0 | connu |
| `ghcr.io/wg-easy/wg-easy:14` (VPN facultatif) | **CC BY-NC-SA 4.0 (non commercial)** jusqu'à la version 14, AGPL-3.0 à partir de la version 15 | **rapporté**, pas lu à la source (les pages du dépôt n'ont pas pu être récupérées) |

**Point à trancher avant de recommander le profil VPN** : la version 14 de wg-easy, que `docker-compose.prod.yml` épingle, serait réservée à un usage non commercial. Pour une installation domestique c'est acceptable, mais le projet ne doit pas la présenter comme librement utilisable par des entreprises. Passer à la version 15 (AGPL-3.0) demande un changement de configuration et un essai ; c'est listé dans [À améliorer](to-improve.md).

## Services externes

| Service | Conditions | Ce que cela implique |
|---|---|---|
| **Open-Meteo** (météo ; `OPEN_METEO_URL`) | L'API gratuite est réservée à un **usage non commercial** (limites : 10 000 appels par jour, 5 000 par heure, 600 par minute) ; les données sont sous **CC BY 4.0, qui exige une attribution** | **rapporté** (page des conditions vue par une recherche). Une installation domestique reste très en dessous des limites. Conservez l'attribution **« Weather data by Open-Meteo.com »** partout où vous republiez les données. Un usage commercial demande une offre payante, ou un autre fournisseur réglé dans `OPEN_METEO_URL` |
| API SMS de Free Mobile | les conditions de l'opérateur, pour votre propre ligne | connu |
| Serveur SMTP, e-mails d'alerte Prometheus/Grafana | les vôtres | |

## Quand vous redistribuez

- Conservez `LICENSE`, `LUCIDE-LICENSE.txt` et l'avis SolarMax de `protocol.rs`.
- Une image Docker publiée contient de l'AGPL (l'application, le modèle), du GPL/LGPL (`ffmpeg`) et des composants permissifs : publiez le source correspondant, c'est-à-dire ce dépôt à l'étiquette dont l'image a été construite.
- N'enlevez pas les en-têtes de licence ; ajoutez `SPDX-License-Identifier: AGPL-3.0-or-later` aux nouveaux fichiers.
