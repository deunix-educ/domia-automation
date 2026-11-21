# ADR 0053 — Relais des alertes Prometheus et tableaux Grafana


Statut : accepté (R7 lot 3).

## Contexte
Les alertes levées par Prometheus doivent parvenir au propriétaire par e-mail (et par SMS pour les critiques), et Grafana a besoin de tableaux écrits pour les métriques de cette application. Le relais lui-même avait deux défauts, et certains tableaux citaient des métriques qui n'existent pas.

## Décisions
- **Relais** (`prometheus_alerts`, rôle `scheduler`, actif seulement si `PROMETHEUS_URL` est renseigné) : toutes les `PROMETHEUS_ALERTS_POLL_SECS` (60 s), `GET /api/v1/alerts`. Une alerte `firing` nouvelle → e-mail « [DomiNet] Alerte » + SMS Free Mobile si `critical` (canaux de R5, règles de D1 : jamais d'envoi réel hors demande) ; silence de `PROMETHEUS_ALERT_COOLDOWN` (3600 s). Le cœur est une **machine à états sans entrée-sortie** (horloge fournie), donc testable et comparable à la référence.
- **Deux écarts voulus avec la référence**, qui étaient des défauts :
  1. la référence indexait son cache par `fingerprint`, champ que l'API d'alertes de **Prometheus ne fournit pas** (c'est celui d'Alertmanager) : toutes les alertes partageaient la même clé et la deuxième se taisait une heure après la première. Rust tire l'identité de l'alerte de **toutes ses étiquettes**.
  2. la référence prévoyait un message « RESOLU » pour l'état `resolved`, que Prometheus ne renvoie jamais (il retire l'alerte de la liste). Rust envoie « RESOLU » (e-mail, jamais de SMS) quand une alerte notifiée **disparaît**, ou passe à `resolved`. C'est un e-mail de plus par alerte qui se termine : dites-le si ce n'est pas souhaité.
- **Messages** identiques à la référence (`[DomiNet — FIRING] nom (SÉVÉRITÉ)`, résumé, description), vérifiés contre le **vrai** `check_prometheus_alerts` de l'image locale passage par passage (alerte en attente ignorée, silence, résolution, sévérité absente, expiration du silence).
- **Tableaux Grafana** : les six tableaux génériques sont portés par un script de portage retiré (SQL : `automation_devicestatelog` → `device_state_log`, etc., y compris les variables de requête ; PromQL : une correspondance explicite par expression, le script échoue sur une expression inconnue). Les tableaux d'équipement (`device_<nom>`) restent produits par l'application (ADR 0029). Les anciens noms MediaMTX (`paths_bytes_received`…) deviennent `paths_inbound_bytes`… (MediaMTX 1.21.3 déclare les premiers obsolètes). La station météo est reconnue à la clé d'état `weather_code` (DomiNet ne renseigne pas le fabricant « Open-Meteo »). Les routes de fonctions serveur (`/sf/<point>`) deviennent une étiquette `route` exacte quand le serveur les connaît (statut ≠ 404), ce qui rend possibles les panneaux « commandes » et « activations de scènes » (complète l'ADR 0051). `dominet_scheduler_task_duration_seconds` est ajoutée pour le panneau de durée des tâches.
- **Fichiers de provisionnement** dans `deploy/` : `grafana/provisioning/{datasources,dashboards,alerting}` (sources PostgreSQL et Prometheus, point de contact e-mail, politique) et `prometheus/prometheus.yml` (jobs `dominet:9100` et `mediamtx:9998`). Le compose (lot 5) les monte.

## Contrôles
- Test d'intégration `grafana_assets` : chaque requête SQL des tableaux (macros et variables remplacées) passe `EXPLAIN` sur le schéma de DomiNet ; **chaque métrique citée** par un tableau ou une règle livrée existe, dans `metrics::names()` ou dans le **relevé du vrai MediaMTX 1.21.3** (`tests/fixtures/observability/mediamtx-1.21.3.names`). Ce test a débusqué `mediamtx_paths`, métrique inexistante de la règle `MediaMTXNoStreams` de la référence (ADR 0052 corrigé).
- `tools/check-observability.sh` : vrais Prometheus 2.52 et Grafana 11.0 jetables sur des ports décalés, base jetable : `promtool check rules`, chargement des règles, rechargement demandé par DomiNet accepté par les deux services, Grafana charge sans erreur tableaux, sources (connexion PostgreSQL réelle), points de contact, politiques et règles d'alerte, et exécute une requête SQL d'un tableau.

## Conséquences
- Les fichiers de règles doivent être sur un volume partagé entre `dominet` (écriture) et Prometheus / Grafana (lecture) : prévu au lot 5.
- un script de portage retiré lit le dépôt de référence en lecture seule ; relancé, il réécrit les six fichiers à l'identique.
