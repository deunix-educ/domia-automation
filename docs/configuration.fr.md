# Configuration

`dominet` se configure uniquement par variables d'environnement (noms repris de `.env.example.prod` de
DomiNet quand ils ont un sens). Toutes les erreurs sont rapportées ensemble au démarrage (code de sortie 2).
Les secrets ne sont jamais affichés ni journalisés.

| Variable | Défaut | Rôle |
|---|---|---|
| `DATABASE_URL` | — (**obligatoire**) | `postgres://utilisateur:motdepasse@hôte:5432/base` |
| `HTTP_ADDR` | `0.0.0.0:8000` | adresse d'écoute HTTP |
| `BACKUP_DIR` | — | dossier des sauvegardes (`postgres/`, `reductstore/`) ; absent : sauvegarde planifiée désactivée |
| `BACKUP_RETENTION_DAYS`, `BACKUP_HOUR` | 7, 3 | âge maximal des sauvegardes (la plus récente reste toujours) ; heure locale de la sauvegarde quotidienne |
| `REDUCTSTORE_BACKUP_SRC` | — | dossier de données de ReductStore, monté en lecture seule, à archiver ; absent : seule la base est sauvegardée |
| `PG_DUMP_BIN`, `PSQL_BIN` | `pg_dump`, `psql` | clients PostgreSQL |
| `PROMETHEUS_RULES_PATH` | — | fichier `rules.yml` que Prometheus relit (règles éditées dans `/admin/prometheus-rules`) ; absent : non écrit |
| `PROMETHEUS_ALERTS_POLL_SECS`, `PROMETHEUS_ALERT_COOLDOWN` | 60, 3600 | sondage de `/api/v1/alerts` (rôle `scheduler`, actif si `PROMETHEUS_URL` est renseigné) et silence d'une alerte déjà notifiée (secondes) |
| `PROMETHEUS_URL` | — | ex. `http://prometheus:9090` : `POST /-/reload` après chaque écriture (Prometheus lancé avec `--web.enable-lifecycle`) |
| `GRAFANA_ALERTING_PATH` | — | fichier `rules.yaml` du provisionnement d'alertes de Grafana (`/admin/grafana-rules`) |
| `GRAFANA_URL`, `GRAFANA_USER`, `GRAFANA_PASSWORD` | —, `admin`, — | rechargement du provisionnement d'alertes de Grafana (compte administrateur) |
| `METRICS_ADDR` | `0.0.0.0:9100` | écouteur `/metrics` pour Prometheus (**jamais publié** sur l'hôte) ; `off` pour le couper ; un port pris n'arrête pas le serveur |
| `DOMINET_MASTER_KEY` | — | clé AES-256 (32 octets en base64) du chiffrement des secrets au repos (`dominet gen-master-key`) ; validée au démarrage ; voir [Réglages](settings.fr.md) |
| `SESSION_IDLE_MINUTES` | `480` | déconnexion après cette durée sans activité |
| `SESSION_MAX_DAYS` | `14` | durée de vie maximale d'une session |
| `MCP_ENABLED` | `false` | active le [serveur MCP](mcp.fr.md) sur `/mcp` (R11) ; les jetons se créent dans Administration › Assistants (MCP) |
| `MCP_RATE_LIMIT_PER_MIN` | `120` | appels par minute et par jeton ; au-delà, 429 |
| `MCP_AUDIT_RETENTION_DAYS` | `365` | durée de conservation du journal d'audit MCP |
| `WS_HEARTBEAT_SECS` | `25` | intervalle des battements WebSocket (1 à 300) ; à chaque battement la session est revalidée |
| `SYSTEM_STATS_SECS` | `5` | intervalle de mesure des ressources de la machine (1 à 3600 s) : jauges de la barre latérale, `/api/system-stats/` |
| `TRUSTED_PROXIES` | — | IP ou réseaux CIDR des proxys dont on croit `X-Forwarded-For` (ex. `172.18.0.1,10.0.0.0/8`) |
| `ALLOWED_HOSTS` | — | hôtes acceptés, séparés par des virgules |
| `CSRF_TRUSTED_ORIGINS` | — | origines de confiance, séparées par des virgules |
| `COOKIE_SECURE` | `true` | `false` pour du HTTP sans TLS sur le LAN |
| `SITE_ROOT` | `target/site` (`/app/site` dans l'image) | dossier des fichiers de l'interface (WASM, JS, CSS) produits par `cargo leptos build` |
| `DOCS_ROOT` | `target/docs-site` (`/app/docs` dans l’image) | site de documentation (MkDocs), servi sur `/docs/` derrière la connexion |
| `CUSTOM_CSS` | — | fichier de personnalisation de l'apparence, servi sur `/custom.css` et relu à chaque chargement (en production : `/custom/custom.css`, monté depuis `deploy/custom/`) : voir [apparence](appearance.fr.md) |
| `TZ` | `Europe/Paris` | fuseau d'affichage |
| `LOG_FORMAT` | `pretty` | `pretty` ou `json` ; le filtre se règle par `RUST_LOG` (défaut `info`) |
| `MQTT_HOST`, `MQTT_PORT`, `MQTT_USER`, `MQTT_PASSWORD` | port `1883` | broker MQTT (utilisé dès R3) ; sans `MQTT_HOST`, le rôle `mqtt` est inactif |
| `MQTT_CLIENT_ID` | `dominet` | identifiant de client MQTT, stable (session persistante) |
| `REDUCTSTORE_URL`, `REDUCTSTORE_TOKEN` | — | ReductStore : archive brute de l'historique (bucket `iot`, dès R3), vidéo (R6) |
| `IOT_QUOTA_GB`, `REDUCTSTORE_IOT_BUCKET` | `5`, `iot` | quota FIFO du bucket `iot` (appliqué à sa création) et son nom |
| `HISTORY_RETENTION_DAYS` | `400` | âge maximal de l'historique PostgreSQL des appareils (purge quotidienne) |
| `GRAFANA_DASHBOARDS_DIR` | — | dossier surveillé par Grafana où sont écrits les tableaux d'appareils (page d'un appareil) ; sans lui, la génération est refusée avec un message |
| `OPEN_METEO_URL`, `SOLARMAX_HOST`, `SOLARMAX_PORT`, `SOLARMAX_INVERTER_ID`, `SOLARMAX_TOPIC`, `SOLARMAX_INTERVAL_SECS` | API publique, —, `12345`, `1`, topic par défaut, `10` | publishers météo et SolarMax : voir [publishers](publishers.fr.md) ; **`SOLARMAX_HOST` n'a pas de défaut** |
| `MEDIAMTX_API_URL` | — | API REST de MediaMTX (ex. `http://mediamtx:9997`) : un *path* par caméra active y est créé, remplacé ou supprimé à chaud, et tout est resynchronisé au démarrage ; sans elle, les caméras s'enregistrent mais rien n'est synchronisé. Les mots de passe des caméras sont chiffrés par `DOMINET_MASTER_KEY` : sans clé, un mot de passe de caméra est refusé |
| `PTZ_DEAD_MAN_SECS` | `8` | un mouvement PTZ continu que personne ne rafraîchit ni n'arrête (onglet figé, réseau coupé) est arrêté par le serveur après ce délai, en plus de l'arrêt à la fermeture de la liaison |
| `MEDIAMTX_RTSP_URL` | — | re-stream RTSP de MediaMTX (ex. `rtsp://mediamtx:8554`) que le rôle `video` décode (`<url>/<uuid>`, 2 images/s, `ffmpeg`) ; **quand elle est renseignée, `dominet serve` lance aussi le rôle `video`**. Sans elle, ni dernière image, ni snapshots, ni repli pour les navigateurs sans H.264 |
| `SNAPSHOTS_DIR`, `SNAPSHOTS_MAX_PER_CAMERA`, `FFMPEG_HWACCEL` | `data/snapshots` (`/data/snapshots` dans l'image), `500`, `auto` | dossier des snapshots (galerie) et de la dernière image de chaque caméra (`latest/`) — **partagé entre les rôles `video` et `web`** s'ils sont deux processus ; au-delà du plafond, les plus anciens snapshots d'une caméra sont supprimés ; décodage VAAPI si disponible (`auto`) ou logiciel (`off`) |
| `RECORDINGS_DIR`, `RECORDINGS_SEGMENT_DURATION`, `RECORDINGS_SCAN_SECS`, `RECORDINGS_GRACE_SECS` | `/recordings`, `60`, `60`, `15` | dossier où MediaMTX écrit les segments fMP4 (`<dossier>/<uuid>/<date>.mp4`, **volume commun avec MediaMTX**), durée d'un segment (celle de `recordSegmentDuration`), intervalle du scan du rôle `video`, délai sans écriture après lequel un segment est considéré comme terminé |
| `VIDEO_QUOTA_GB`, `REDUCTSTORE_VIDEO_BUCKET` | `50`, `surveillance` | quota FIFO (Go) et nom du bucket ReductStore des segments vidéo ; sans `REDUCTSTORE_URL`, les segments restent sur le disque |
| `MOTION_THRESHOLD`, `MOTION_INTERVAL_SECS`, `MOTION_ALERT_COOLDOWN`, `DETECTION_ALERT_COOLDOWN` | `0.01`, `30`, `600`, `300` | mouvement (caméras en mode `motion`) : part de l'image qui doit bouger, intervalle d'évaluation, délais minimaux entre deux notifications d'une même caméra (mouvement, détection)  |
| `YOLO_MODEL_PATH`, `YOLO_WORKERS`, `DETECTIONS_DIR` | `models/yolov8n.onnx` (`/models/yolov8n.onnx` dans l'image), `1`, `data/detections` (`/data/detections`) | modèle de détection (`make fetch-model`, non commité), sessions ONNX Runtime à un fil chacune, dossier des images annotées. Le détecteur s'active dans l'interface (`/surveillance/events`) |
| `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS`, `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS`, `SMS_FREE_URL` | —, `587` (`465` en SSL), —, —, `true`, `false`, `10`, API officielle | envoi d'e-mails (noms usuels) et adresse de l'API Free Mobile : voir [notifications](notifications.fr.md). **Sans `EMAIL_HOST`, aucun e-mail.** Des identifiants sans TLS ne sont admis que vers la boucle locale |

## Commandes

```text
dominet serve [--only web,mqtt,...]   démarre le serveur (seul `web` existe pour l'instant)
dominet migrate | seed [--dry-run] | gen-master-key | createsuperuser | passwd
dominet healthcheck [--addr h:p]      sonde GET /healthz (code 0 si 200) : healthcheck Docker, sans base ni configuration complète
```

## Sondes

- `GET /healthz` : 200 tant que le processus répond (aucune dépendance).
- `GET /readyz` : 200 si PostgreSQL répond en moins de 2 s, sinon 503 (sans détail sur la cause).
- Chaque réponse porte un en-tête `x-request-id` (repris de la requête s'il existe), présent dans les journaux.
