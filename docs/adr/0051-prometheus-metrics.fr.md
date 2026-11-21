# ADR 0051 — Métriques Prometheus (`/metrics`)


Statut : accepté (R7 lot 1, décision D2 du plan).

## Contexte
Prometheus doit scruter l'application (15 s) avec des règles sur `up`, les erreurs 5xx, la latence p95 et l'échec des tâches, sans exposer de données aux visiteurs : `/metrics` n'est donc pas servi sur le port public.

## Options
1. **Route `/metrics` sur le port public**, protégée par un jeton : un secret de plus, et une page de plus derrière nginx.
2. **Écouteur séparé** (`METRICS_ADDR`), joint par Prometheus sur le réseau Docker, jamais publié sur l'hôte ni proxifié.
3. Bibliothèque : `metrics` + `metrics-exporter-prometheus` (sans son écouteur intégré) ou `prometheus-client`.

## Décision
- Option 2 : **`METRICS_ADDR`**, défaut `0.0.0.0:9100`, `off` pour couper ; un port déjà pris est signalé et n'arrête pas le serveur (plusieurs processus sur un poste de dev). Le routeur public ne connaît pas `/metrics` (404, testé).
- **`metrics` + `metrics-exporter-prometheus`** sans écouteur : un enregistreur global installé une fois (`metrics::handle()`), l'écouteur est le nôtre.
- Noms `dominet_*` (convention Prometheus) : `dominet_http_requests_total{method,route,status}`, `dominet_http_request_duration_seconds` (histogramme 5 ms à 30 s), `dominet_http_requests_in_flight`, `dominet_ws_connections`, `dominet_mqtt_messages_received_total`, `dominet_mqtt_disconnections_total`, `dominet_mqtt_connected`, `dominet_scheduler_tasks_total{task,result}`, `dominet_recordings_total{outcome}`, `dominet_motion_events_total{camera}`, `dominet_detection_events_total{camera}`, `dominet_db_pool_connections{state}`, `dominet_build_info{version}`, `dominet_process_start_time_seconds`.
- **Étiquettes bornées, rien de sensible** : la route est le *modèle* de route (`MatchedPath`) ou une famille fixe (`/sf/*`, `/pkg/*`, `/api/*`, `ui`), jamais l'URL brute (identifiants, jeton Android) ; aucun identifiant de compte ; la caméra est identifiée par son UUID public. Testé.
- Les jauges de pool et de MQTT sont rafraîchies toutes les 15 s et juste avant chaque lecture.

## Conséquences
- Les règles Prometheus du lot 2 s'écrivent sur ces noms (exemples : taux de 5xx → `dominet_http_requests_total{status=~"5.."}`, latence p95 → `histogram_quantile` sur `dominet_http_request_duration_seconds_bucket`, échecs du planificateur → `dominet_scheduler_tasks_total{result="error"}`).
- Un processus par rôle a ses propres métriques (le service `dominet-discovery`, en réseau hôte, doit utiliser `METRICS_ADDR=off` pour ne rien exposer sur le LAN).
- La bibliothèque `metrics` est une simple façade (`dominet-mqtt` l'utilise aussi) ; sans enregistreur installé, les appels ne coûtent presque rien.
