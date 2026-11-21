# ADR 0052 — Règles d'alerte Prometheus et Grafana


Statut : accepté (R7 lot 2, décision D3 du plan).

## Contexte
Dans la référence, les règles d'alerte de Prometheus (`AlertRule`) et de Grafana (`GrafanaAlertRule`) sont des lignes de base éditées dans l'administration ; l'application en tire les fichiers de provisionnement (`rules.yml`, `rules.yaml`) puis demandent aux services de les relire. Il y a déjà une table `alert_rule` en Rust : ce sont les règles sur l'**état des appareils** (R4), sans rapport.

## Options
1. **Règles en base, éditables** (parité avec la référence), fichiers régénérés et services rechargés.
2. Fichiers statiques versionnés : plus simple, mais plus d'édition en ligne.
3. Un seul modèle pour les deux services : les champs diffèrent (SQL brut, états sans données / erreur, dossier).

## Décision
- Option 1, deux tables (`prometheus_rule`, `grafana_alert_rule`, migration 0012) : les contraintes de la référence sont reprises en `CHECK` (sévérité, états, uid en `[a-z0-9_-]`), et validées aussi par le domaine champ par champ.
- **Documents** : `prometheus_rules_document` / `grafana_rules_document` (domaine, fonctions pures) construisent exactement ceux de la référence : regroupement par groupe (Prometheus) ou (dossier, groupe, intervalle) (Grafana), **intervalle du groupe = celui de la première règle**, source ≠ `prometheus` lue en **SQL brut** avec une fenêtre d'au moins 60 s. Rendu YAML maison (blocs, chaînes entre guillemets) : le fichier se relit comme le document.
- **Écriture et rechargement** (`RulesService`) : fichier temporaire puis renommage (jamais de fichier à moitié écrit, ce que la référence ne garantissait pas) ; `POST {PROMETHEUS_URL}/-/reload` ; `POST {GRAFANA_URL}/api/admin/provisioning/alerting/reload` avec le compte `GRAFANA_USER` / `GRAFANA_PASSWORD`. Un rechargement refusé ou injoignable est **signalé, pas fatal** : le fichier reste écrit. Chemins et adresses sont facultatifs (`PROMETHEUS_RULES_PATH`, `PROMETHEUS_URL`, `GRAFANA_ALERTING_PATH`, `GRAFANA_URL`) ; absents, le service est « non configuré ».
- **Quand** : après chaque enregistrement ou suppression dans l'administration, au démarrage du rôle web, à la demande (bouton de la page, `dominet sync-rules`).
- **Règles livrées** : `dominet seed` crée les règles portées de la référence (noms de métriques de DomiNet, ADR 0051 ; table `device` au lieu de `automation_device`), **seulement dans une table vide** : une règle que l'administrateur a supprimée ne revient pas. Ajout : `DominetMqttDown` (client MQTT déconnecté 5 min). Les règles un planificateur externe disparaissent (`SchedulerTaskFailureRate` les remplace).
- `dump-context` liste les règles Grafana actives (tri source puis uid).

## Conséquences
- Un résultat de référence figé a été produit une fois sur des entrées variées ; les tests Rust rejouent les mêmes entrées et comparent les documents produits.
- `MediaMTXNoStreams` : l'expression de la référence (`mediamtx_paths == 0`) citait une métrique qui **n'existe pas** (relevé sur MediaMTX 1.21.3, ADR 0053) et ne se déclenchait donc jamais ; elle devient `(sum(paths{job="mediamtx",state="ready"}) or vector(0)) == 0`.
- Les règles Grafana sur PostgreSQL lisent `device.state` : leur exécution contre le schéma Rust est contrôlée au lot 3.
