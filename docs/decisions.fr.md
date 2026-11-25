# Décisions d'architecture : index par thème

Les fiches complètes sont les ADR de [`adr/`](adr/README.md) (en anglais, avec la version française `*.fr.md`) (contexte, options, décision, conséquences). Cette page est la carte : ce qui a été décidé dans chaque domaine, les décisions **renversées ou abandonnées** et pourquoi, et ce qui est volontairement repoussé.

## Par thème

### Architecture et construction
Un binaire à rôles ; Leptos en rendu serveur avec WebSocket et traductions Fluent ; fonctions serveur sous `/sf/` ; un composant CRUD générique pour les pages d'administration ; une image construite avec `cargo-leptos` ; un garde-fou qui relance un rendu figé ; tests navigateur sur Marionette/Firefox.

ADR : [0001](adr/0001-global-architecture.md), [0007](adr/0007-leptos-ssr-frontend.md), [0012](adr/0012-interface-architecture.md), [0014](adr/0014-generic-crud.md), [0017](adr/0017-docker-image-and-e2e.md), [0021](adr/0021-page-render-guard.md), [0024](adr/0024-e2e-marionette.md).

### Données et stockage
ReductStore pour l'historique brut, PostgreSQL pour l'état courant et les agrégats ; un état remplacé plus un historique à la seconde ; images en base ; secrets chiffrés au repos en AES-256-GCM ; `pg_dump` et archive ReductStore pour les sauvegardes.

ADR : [0002](adr/0002-iot-storage.md), [0018](adr/0018-dashboard-schema.md), [0025](adr/0025-device-schema.md), [0028](adr/0028-mqtt-sources-and-raw-archive.md), [0011](adr/0011-secrets-encryption.md), [0054](adr/0054-backups.md).

### Sécurité et accès
Sessions côté serveur avec argon2id ; CSP à nonce ; MQTT avec un seul listener authentifié et des comptes par client ; jetons MCP hachés, à portées, révocables, avec journal d'audit ; visionnage des caméras autorisé par DomiNet (jeton de visionnage dans le navigateur, mot de passe personnel révocable pour les lecteurs externes). Le VPN facultatif est wg-easy 15 (AGPL-3.0), réglé une seule fois par son API.

ADR : [0004](adr/0004-auth-sessions.md), [0016](adr/0016-csp-and-websocket-hub.md), [0057](adr/0057-authenticated-mqtt.md), [0059](adr/0059-mcp-access-tokens.md), [0064](adr/0064-authenticated-camera-viewing.md), [0065](adr/0065-wg-easy-15.md).

### Temps réel et MQTT
`rumqttc` ; sujets WebSocket typés qui gardent le dernier état ; client supervisé qui publie tout de suite ou refuse ; ingestion Zigbee2MQTT ; publishers météo et solaire supervisés.

ADR : [0005](adr/0005-mqtt-client-rumqttc.md), [0019](adr/0019-multiplexed-websocket-protocol.md), [0020](adr/0020-system-statistics.md), [0026](adr/0026-supervised-mqtt-client.md), [0027](adr/0027-zigbee2mqtt-ingestion.md), [0030](adr/0030-weather-and-solarmax-publishers.md).

### Automations, alertes et énergie
Rhai isolé ; stocké dans la forme JSON des scénarios ; un moteur, une file ; un planificateur unique avec événements solaires ; évaluation et cycle de vie des alertes ; bilan d'énergie journalier ; alertes sur les canaux réels.

ADR : [0006](adr/0006-rhai-sandbox.md), [0031](adr/0031-automation-schema.md), [0032](adr/0032-automation-engine.md), [0033](adr/0033-scheduler.md), [0034](adr/0034-rhai-scripts.md), [0035](adr/0035-alerts.md), [0036](adr/0036-energy.md), [0042](adr/0042-alerts-on-real-channels.md).

### Messagerie et relais téléphone
Canaux SMTP et SMS dans une crate à part ; le relais Android garde son contrat WebSocket ; jetons d'appareil hachés et affichés une fois ; envoi asynchrone avec statuts en direct.

ADR : [0037](adr/0037-android-relay-and-messaging.md), [0038](adr/0038-delivery-channels.md), [0039](adr/0039-android-relay-server.md), [0040](adr/0040-android-pages.md), [0041](adr/0041-asynchronous-messaging.md).

### Surveillance et vidéo
`ffmpeg` en sous-processus supervisé derrière un trait `FrameSource` ; client ONVIF minimal écrit à la main ; YOLOv8n par ONNX Runtime ; live WHEP en WASM via MediaMTX ; MediaMTX enregistre et l'application indexe ; détection de mouvement en Rust pur.

ADR : [0003](adr/0003-video-decoding.md), [0008](adr/0008-minimal-onvif-client.md), [0009](adr/0009-yolo-ort.md), [0010](adr/0010-whep-live-wasm.md), [0043](adr/0043-surveillance-schema.md), [0044](adr/0044-mediamtx-and-cameras.md), [0045](adr/0045-onvif-and-discovery.md), [0046](adr/0046-whep-live-view.md), [0047](adr/0047-ptz-and-night-mode.md), [0048](adr/0048-decoded-frames-and-snapshots.md), [0049](adr/0049-recording-and-replay.md), [0050](adr/0050-motion-and-detection.md).

### Observabilité et exploitation
Un point `/metrics` ; règles Prometheus et Grafana stockées en base ; relais des alertes externes ; base Compose et surcouches ; MkDocs servi par l'application ; barrière locale avant push ; alertes de plateforme en lecture seule dans la page des alertes.

ADR : [0013](adr/0013-self-hosted-ci.md), [0015](adr/0015-local-gate.md), [0051](adr/0051-prometheus-metrics.md), [0052](adr/0052-prometheus-and-grafana-alert-rules.md), [0053](adr/0053-prometheus-alerts-and-grafana-dashboards.md), [0055](adr/0055-compose-base-and-overlays.md), [0056](adr/0056-mkdocs-documentation.md), [0061](adr/0061-platform-alerts-in-the-alerts-page.md).

### MCP et assistant local
Un serveur MCP intégré au binaire ; jetons à portées ; un modèle local qui propose et une application qui décide.

ADR : [0058](adr/0058-mcp-server.md), [0059](adr/0059-mcp-access-tokens.md), [0060](adr/0060-local-assistant.md).

## Décisions renversées, remplacées ou abandonnées

| Décision | Ce qui s'est passé | Pourquoi |
|---|---|---|
| CI sur GitHub Actions (au départ) | Remplacée par un runner auto-hébergé ([0013](adr/0013-self-hosted-ci.md)), puis par une barrière locale : le hook `pre-push` relance `make ci` sur une copie propre ([0015](adr/0015-local-gate.md)). Le dépôt public ne garde que la barrière locale | Le workflow ne faisait que relancer `make ci`, que le développeur lance déjà sur la même machine (aussi serveur de développement et de production) : 8 à 10 minutes par push, un utilisateur système dédié, Docker sans root, des plafonds de ressources à entretenir, des échecs difficiles à lire ou à reproduire. Un seul développeur n'y gagne rien |
| Deux listeners Mosquitto, dont un anonyme sur le LAN ([0055](adr/0055-compose-base-and-overlays.md)) | Un seul listener avec mots de passe et comptes par client ([0057](adr/0057-authenticated-mqtt.md)) | La nouvelle pile s'installe à neuf, sans client MQTT existant à ménager ; un listener anonyme, même désactivé par défaut, rouvre le courtier au LAN au moindre oubli de configuration |
| Deux modèles d'assistant, 8B au quotidien et 14B pour la configuration ([0060](adr/0060-local-assistant.md)) | Un seul Qwen3-8B | Mesuré à 98 % contre 96 %, trois fois plus rapide |
| Un assistant qui renomme et modifie des éléments existants | Abandonné ; l'assistant lit, pilote et crée | Peu fiable avec le 8B, et le nom d'un appareil est aussi son topic MQTT ; l'administration le fait mieux |
| La phase de l'assistant local (R12) | Mise de côté le 2026-11-18 après essais | Lecture, pilotage et création fiables ; modification non |
| Migrer `chrono` vers `jiff` ou `time` | Rejeté le 2026-11-20 | `sqlx` 0.9 n'a pas de support `jiff`, et `rmcp` et `schemars` gardent `chrono` dans l'arbre de toute façon. À rouvrir seulement sur avis de sécurité, support `jiff` de `sqlx`, ou besoin que `chrono` ne couvre pas |
| Leptos 0.9 ([0007](adr/0007-leptos-ssr-frontend.md)) | Resté en 0.8.x | `leptos-fluent` exigeait moins de 0.9 à l'époque ; à réévaluer quand la 0.9 sera stable et prise en charge |
| Un décodeur H.264 en Rust pur ou lié | Rejeté : sous-processus `ffmpeg` ([0003](adr/0003-video-decoding.md)) | Aucun décodeur Rust pur mature n'existe (`retina` ne décode pas) ; `ffmpeg-next` exige les en-têtes de développement ; `openh264` coûtait environ deux fois plus de processeur et a échoué sur deux familles de caméras. Mesuré : 4 caméras à 22 % de la machine en logiciel, 8 % avec VAAPI |
| Une liaison OpenCV pour le mouvement | Rejetée : mouvement en Rust pur ([0050](adr/0050-motion-and-detection.md)) | Aucune dépendance à OpenCV ; l'algorithme et les détails propres d'OpenCV sont reproduits et la parité est mesurée sur des paires d'images synthétiques |
| Une crate ONVIF | Rejetée : client SOAP minimal écrit à la main ([0008](adr/0008-minimal-onvif-client.md)) | Aucune crate ONVIF mature ne convient ; il faut peu d'appels, et les familles de caméras utilisent des préfixes de noms différents, les réponses sont donc lues sans s'y fier |
| Remplacer MediaMTX par une pile vidéo tout-Rust | Repoussé (phase optionnelle R9) | N'a de sens que si les mesures montrent un gain réel |
| Acquittement des alertes de plateforme | Non proposé : lecture seule ([0061](adr/0061-platform-alerts-in-the-alerts-page.md)) | Choix du propriétaire (lecture seule, un seul total dans la pastille) ; l'acquittement reste dans les outils qui possèdent ces alertes |

## Décidé et pas encore fait

- Une image arm64 n'a jamais été construite ([0017](adr/0017-docker-image-and-e2e.md)) ; nécessaire pour le Raspberry Pi 5.
- L'envoi de SMS est volontairement exclu des outils MCP.
- L'assistant vocal, la vidéo tout-Rust (R9) et les extensions matérielles (R10) sont des phases ultérieures.

## Conventions apprises à leurs dépens

- Toute décision non évidente reçoit un ADR avec les options écartées.
- Ne jamais publier avec `-C target-cpu=native` ; la base est `x86-64-v2` ([0001](adr/0001-global-architecture.md)).
- Rien ne s'inclut depuis l'extérieur de `crates/` (l'image ne copie que cela et `.sqlx/`).
- Un script n'atteint le système que par la portée `administration` ([0059](adr/0059-mcp-access-tokens.md)).
