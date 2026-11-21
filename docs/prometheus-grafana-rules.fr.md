# Règles d'alerte Prometheus et Grafana

Deux pages d'administration (réservées aux administrateurs) : **Règles Prometheus** (`/admin/prometheus-rules`) et **Règles Grafana** (`/admin/grafana-rules`). Les règles sont en base ; chaque modification **réécrit le fichier de provisionnement** (`rules.yml` pour Prometheus, `rules.yaml` pour Grafana) puis **demande au service de le relire**. Ces règles surveillent la **plateforme** (application, MQTT, MediaMTX…) ; les règles sur l'état des appareils sont ailleurs (`/admin/alert-rules`, voir [alertes](alerts.fr.md)).

## Règles livrées
`dominet seed` crée les règles par défaut, écrites sur les métriques de DomiNet ([`/metrics`](configuration.fr.md), ADR 0051) : tâches planifiées en échec, application hors ligne, erreurs 5xx, latence p95, MQTT déconnecté, MediaMTX hors ligne ou sans flux ; côté Grafana : canicule (SQL sur `device`), application et MediaMTX hors ligne, erreurs 5xx. Elles ne sont créées **que dans une table vide** : une règle supprimée ne revient pas.

## Champs
- **Prometheus** : nom (unique), groupe, sévérité, expression PromQL, délai avant alerte (`5m`), résumé, description (variables `{{ $value }}`, `{{ $labels.job }}`), intervalle d'évaluation du groupe (celui de la **première** règle du groupe), active.
- **Grafana** : titre, identifiant (UID, tiré du titre si vide), dossier, groupe, intervalle, source de données (`prometheus` : PromQL ; autre, par exemple `postgresql` : **SQL brut**), requête, fenêtre (secondes), délai, sévérité, résumé, description, état « pas de données » et « erreur d'exécution », active.
- Durées : `30s`, `5m`, `1h30m` (unités `ms s m h d w y`).

## Fichiers et rechargement
| Variable | Rôle |
|---|---|
| `PROMETHEUS_RULES_PATH`, `PROMETHEUS_URL` | fichier `rules.yml` ; `POST /-/reload` |
| `GRAFANA_ALERTING_PATH`, `GRAFANA_URL`, `GRAFANA_USER`, `GRAFANA_PASSWORD` | fichier `rules.yaml` ; `POST /api/admin/provisioning/alerting/reload` |

Le fichier est écrit **de façon atomique**. Si le rechargement échoue (service arrêté, refus), le fichier reste écrit et la page le dit ; le service le lira à son prochain démarrage. Sans chemin configuré, le service est « non configuré » (aucune écriture). `dominet sync-rules` fait la même chose en ligne de commande ; l'écriture a aussi lieu au démarrage du serveur.

## État « sans données » des règles livrées
Les quatre règles Grafana livrées ont l'état « sans données » à **`OK`**. Une requête de type `up == 0` ou « taux de 5xx » ne renvoie rien tant que tout va bien : avec `NoData`, Grafana envoyait à chaque évaluation une alerte d'anomalie, constatée à l'installation de production (R8). Si vous créez vos propres règles de ce type, choisissez `OK`. Une installation déjà faite avec l'ancien réglage se corrige dans `/admin/grafana-rules` (les règles existantes ne sont jamais modifiées par `dominet seed`).

## Vérification
Les fichiers produits sont comparés à des valeurs de référence figées. Voir l'[ADR 0052](adr/0052-prometheus-and-grafana-alert-rules.md).
