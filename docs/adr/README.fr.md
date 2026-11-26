# Décisions d'architecture (ADR)

Format : `0000-template.md` (contexte, options, décision, conséquences). La version de référence des ADR est l'anglais lorsqu'il existe ; les documents qui les citent ont une version
anglaise. Un ADR remplacé n'est pas supprimé : son statut renvoie vers celui qui le remplace.

| ADR | Sujet | Statut | Phase |
|---|---|---|---|
| [0001](0001-global-architecture.md) | Architecture globale (workspace, binaire unique, cible `x86-64-v2`) | accepté | R0 |
| [0002](0002-iot-storage.md) | Stockage IoT : ReductStore (brut) + PostgreSQL (agrégats) | accepté | R0, spike 2 |
| [0003](0003-video-decoding.md) | Décodage vidéo : `ffmpeg` en sous-processus derrière `FrameSource` | accepté | R0, spike 4 |
| [0004](0004-auth-sessions.md) | Authentification et sessions (store maison sur `sqlx` 0.9) | accepté | R0 / R1 lot 5 |
| [0005](0005-mqtt-client-rumqttc.md) | Client MQTT : `rumqttc` | accepté | R0, spike 8 |
| [0006](0006-rhai-sandbox.md) | Scripts d'automation : hôte Rhai sandboxé | accepté | R0, spike 7 |
| [0007](0007-leptos-ssr-frontend.md) | Front : Leptos SSR + hydratation, WebSocket, i18n Fluent | accepté | R0, spike 1 |
| [0008](0008-minimal-onvif-client.md) | ONVIF : client SOAP minimal maison | accepté | R0, spike 6 |
| [0009](0009-yolo-ort.md) | Détection d'objets : YOLOv8n via `ort` | accepté (réserves) | R0, spike 5 |
| [0010](0010-whep-live-wasm.md) | Live caméra : client WHEP en Rust/WASM vers MediaMTX | accepté | R0, spike 3 |
| [0011](0011-secrets-encryption.md) | Chiffrement des valeurs sensibles au repos (AES-256-GCM) | accepté | R1 lot 6 |
| [0012](0012-interface-architecture.md) | Interface : fonctions serveur Leptos et trois crates | accepté | R1 lot 7 |
| [0013](0013-self-hosted-ci.md) | CI sur un runner auto-hébergé | **partiellement remplacé par 0015** | R1 |
| [0014](0014-generic-crud.md) | Composant CRUD générique des pages d'administration | accepté | R1 lot 7 |
| [0015](0015-local-gate.md) | Barrière locale (hook `pre-push`) à la place de la CI automatique | accepté | R1 |
| [0016](0016-csp-and-websocket-hub.md) | CSP à nonce et hub WebSocket de base | accepté | R1 lot 8 |
| [0017](0017-docker-image-and-e2e.md) | Image Docker avec `cargo-leptos` et E2E sur l'image | accepté | R1 lot 9 |
| [0018](0018-dashboard-schema.md) | Schéma du tableau de bord : exclusion de chevauchement, références sans clé étrangère | accepté | R2 lot 1 |
| [0019](0019-multiplexed-websocket-protocol.md) | Protocole WebSocket multiplexé : sujets typés, dernier état conservé | accepté | R2 lot 2 |
| [0020](0020-system-statistics.md) | Statistiques système : `sysinfo`, publiées comme dernier état du sujet `system` | accepté | R2 lot 3 |
| [0021](0021-page-render-guard.md) | Garde-fou du rendu des pages : abandon et relance d'un rendu figé (défaut de Leptos 0.8.23) | accepté (contournement) | R2 lot 3 |
| [0022](0022-dashboard-read-view.md) | Tableau de bord en lecture : positions par classes, HTML assaini, iframes Grafana limitées | accepté | R2 lot 4 |
| [0023](0023-dashboard-editing.md) | Édition des tableaux : administrateurs seulement, glisser au pointeur, formulaire à champs conditionnels | accepté | R2 lot 5 |
| [0024](0024-e2e-marionette.md) | E2E navigateur : rester sur Marionette/Firefox | accepté | R2 lot 6 |
| [0025](0025-device-schema.md) | Schéma des appareils : état remplacé, historique à la seconde, images en base | accepté | R3 lot 1 |
| [0026](0026-supervised-mqtt-client.md) | Client MQTT supervisé : publications immédiates ou refusées, resouscription à chaque connexion | accepté | R3 lot 2 |
| [0027](0027-zigbee2mqtt-ingestion.md) | Ingestion Zigbee2MQTT : même résultat que la référence, vérifié en exécutant le code de référence | accepté | R3 lot 3 |
| [0028](0028-mqtt-sources-and-raw-archive.md) | Sources MQTT dynamiques, archive brute ReductStore non bloquante, rétention de l'historique | accepté | R3 lot 4 |
| [0029](0029-device-commands-and-pages.md) | Commandes validées, pages et images des appareils, tableau Grafana en fichier | accepté | R3 lot 5 |
| [0030](0030-weather-and-solarmax-publishers.md) | Publishers météo et SolarMax supervisés ; protocole vérifié contre la référence, onduleur jamais contacté | accepté | R3 lot 6 |
| [0031](0031-automation-schema.md) | Schéma des scènes, automations, alertes et énergie : JSON de la référence, types forts, écarts voulus | accepté | R4 lot 1 |
| [0032](0032-automation-engine.md) | Moteur d'automations : file unique, événements utiles seulement, états regroupés toujours vus, anti-boucle 30/min | accepté | R4 lot 3 |
| [0033](0033-scheduler.md) | Planificateur : rôle `scheduler`, pas de rattrapage, cron comme `TzAwareCrontab`, calcul solaire vérifié contre `ephem` | accepté | R4 lot 4 |
| [0034](0034-rhai-scripts.md) | Scripts Rhai : tout ou rien, clé absente = `()`, code de source mis en sourdine, la référence refusé, essai à blanc | accepté | R4 lot 5 |
| [0035](0035-alerts.md) | Alertes : cycle de la référence avec ses particularités (vérifié sur 2 030 évaluations et 8 scénarios), port `Notifier`, sujet `alerts` | accepté | R4 lot 6 |
| [0036](0036-energy.md) | Bilan d'énergie : même calcul que la référence (13 jours identiques), jour local, SVG côté serveur, `seed-energy` / `aggregate-energy` | accepté | R4 lot 7 |
| [0037](0037-android-relay-and-messaging.md) | Relais Android et messagerie : jeton haché, file `sms_outbox` (accusés), journal d'appels, états d'envoi | accepté | R5 lot 1 |
| [0038](0038-delivery-channels.md) | Canaux d'envoi : `lettre` (STARTTLS obligatoire, identifiants en clair refusés hors boucle locale), Free Mobile vérifié, faux serveurs, `notify-test` | accepté | R5 lot 2 |
| [0039](0039-android-relay-server.md) | Relais Android côté serveur : point d'accès exact, une connexion par appareil, battement 30/90 s, accusés 60 s, sujet `android:<id>`, vérifié contre le consommateur de référence | accepté | R5 lot 3 |
| [0040](0040-android-pages.md) | Pages Android : droits par compte, jeton affiché une fois, relecture en direct, état d'envoi des SMS, téléphone simulé pour l'E2E | accepté | R5 lot 4 |
| [0041](0041-asynchronous-messaging.md) | Messagerie : envoi asynchrone en tâche de fond, statuts par destinataire en direct (`message:<id>`), droits, reprise après redémarrage | accepté | R5 lot 5 |
| [0042](0042-alerts-on-real-channels.md) | Alertes et notifications d'automation sur les canaux réels : trois canaux de la règle, isolation des échecs, `notified` après essai, e-mail (+ SMS si demandé) pour l'action `notification` | accepté | R5 lot 6 |
| [0043](0043-surveillance-schema.md) | Schéma de la surveillance : sept tables, identifiant public de 32 hexadécimaux, secrets chiffrés, ports par agrégat | accepté | R6 lot 1 |
| [0044](0044-mediamtx-and-cameras.md) | Client MediaMTX (REST seul, `replace`, paths gérés = 32 hexadécimaux), service des caméras (secrets chiffrés, échec de MediaMTX non bloquant), droits | accepté | R6 lot 2 |
| [0045](0045-onvif-and-discovery.md) | ONVIF (client, adresses de services, secrets masqués), découverte par `LISTEN`/`NOTIFY` entre les rôles `discovery` et `web`, ajout par adresse | accepté | R6 lot 3 |
| [0046](0046-whep-live-view.md) | Live d'une caméra : lecteur WHEP Rust/WASM, reconnexion par `getStats`, message sans H.264, adresse WHEP adaptée à l'hôte, `connect-src`, test avec un vrai flux H.264 | accepté | R6 lot 4 |
| [0047](0047-ptz-and-night-mode.md) | PTZ ONVIF (protocole de la référence, arrêt automatique, presets sans orphelins), quatre méthodes jour / nuit derrière `NightControl`, droits administrateurs | accepté | R6 lot 5 |
| [0048](0048-decoded-frames-and-snapshots.md) | `FrameSource` (un `ffmpeg` supervisé par caméra, JPEG), rôle `video`, dernière image sur disque, snapshots indexés avec plafond, repli sans H.264 | accepté | R6 lot 6 |
| [0049](0049-recording-and-replay.md) | Enregistrement par MediaMTX (fMP4), scan des segments terminés, archive ReductStore avec les étiquettes de la référence et suppression du fichier local, replay avec `Range`, enregistrement manuel | accepté | R6 lot 7 |
| [0050](0050-motion-and-detection.md) | Mouvement (algorithme de la référence en Rust pur, parité OpenCV mesurée), fenêtre glissante du `FrameHub`, YOLO via `ort` (parité 17/17), modèle non commité (`make fetch-model`), page des événements | accepté | R6 lot 8 |
| [0051](0051-prometheus-metrics.md) | Métriques Prometheus : écouteur `/metrics` séparé (`METRICS_ADDR`), `metrics` + exporter sans écouteur, étiquettes bornées | accepté | R7 lot 1 |
| [0052](0052-prometheus-and-grafana-alert-rules.md) | Règles d'alerte Prometheus et Grafana en base, fichiers de provisionnement atomiques, rechargement signalé non fatal, graine seulement en table vide | accepté | R7 lot 2 |
| [0053](0053-prometheus-alerts-and-grafana-dashboards.md) | Relais des alertes Prometheus (machine à états, deux défauts de la référence corrigés), tableaux Grafana portés, contrôles par exécution et par vrais services | accepté | R7 lot 3 |
| [0054](0054-backups.md) | Sauvegardes : pg_dump gzippé + archive du volume ReductStore, sommes SHA-256, restauration en une transaction, rétention, planifiée, mesurée | accepté | R7 lot 4 |
| [0055](0055-compose-base-and-overlays.md) | Compose : base de 8 services + surcouches dev/prod/demo, images figées, sonde `dominet healthcheck`, Mosquitto à deux listeners, volumes partagés | accepté (listener LAN anonyme remplacé par 0057) | R7 lot 5 |
| [0056](0056-mkdocs-documentation.md) | Documentation MkDocs FR/EN construite dans l'image, servie sur `/docs/` derrière la connexion (`DOCS_ROOT`) | accepté | R7 lot 7 |
| [0057](0057-authenticated-mqtt.md) | MQTT non anonyme : un seul listener avec mot de passe, comptes nominatifs par client, droits par sujet (`make prod-mqtt-user`) | accepté | R8 lot 2 |
| [0058](0058-mcp-server.md) | Serveur MCP intégré au binaire : `rmcp` 3.10.0 épinglé, HTTP sans session, crate `dominet-mcp`, script Rhai en portée `administration` | accepté | R11 lot M0 |
| [0059](0059-mcp-access-tokens.md) | Jetons d'accès MCP : hachés, à portées, révocables, expiration obligatoire, journal d'audit | accepté | R11 lot M0 |
| [0060](0060-local-assistant.md) | Assistant local : `llama-server`, Qwen3 8B (quotidien) et 14B (configuration), mesures du bench | accepté | R12 lot A0 |
| [0061](0061-platform-alerts-in-the-alerts-page.md) | Les alertes de Prometheus et de Grafana dans la page des alertes : lecture seule, une pastille | accepté | R13 lot E1 |
| [0062](0062-published-images-with-local-build-fallback.md) | Images multi-architecture publiées, construction locale en repli | accepté |
| [0063](0063-arm64-support-levels.md) | arm64 et Raspberry Pi 5 : prise en charge au mieux, état de test explicite | accepté |
| [0064](0064-authenticated-camera-viewing.md) | Visionnage des caméras authentifié : MediaMTX interroge DomiNet, jetons de visionnage par compte, identifiants de flux personnels | implémenté |
| [0065](0065-wg-easy-15.fr.md) | VPN : wg-easy 15 (AGPL-3.0) en réseau hôte, première configuration par son API | implémenté |
| [0066](0066-android-application.fr.md) | L'application Android (DomoComm) dans le dépôt, APK signé sur GitHub | implémenté |

Décisions prises hors ADR (voir `PROJECT_CONTEXT.md`) : langues, toolchain épinglée, thème sombre seul, migrations par commande séparée, Mosquitto anonyme.
