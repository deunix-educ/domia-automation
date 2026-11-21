# ADR 0061 — Les alertes de Prometheus et de Grafana dans la page des alertes


Statut : accepté (R13 lot E1, décisions de l'utilisateur du 2026-11-18).

## Contexte
Trois systèmes d'alertes coexistent : les règles d'**appareil** (base de DomiNet, page `/alerts`, pastille de la barre latérale), les règles **Prometheus** (relayées par e-mail et SMS, ADR 0053) et les règles **Grafana** (point de contact `dominet-email`). Seules les premières se voyaient dans l'interface : une alerte de la plateforme (MQTT hors ligne, erreurs 5xx, canicule) n'apparaissait que dans un message.

## Options
- **A. Tout stocker en base** (historique, acquittement). Beaucoup de code (migration, règle de réouverture) pour un besoin de lecture.
- **B. Lire à chaque affichage de page.** Aucun état, mais la pastille ne bouge pas sans visite de la page, et chaque page ouverte appelle Prometheus et Grafana.
- **C. Lire en fond, garder le dernier instantané en mémoire.** Une lecture toutes les `PROMETHEUS_ALERTS_POLL_SECS` (60 s), pastille diffusée à chaque changement.

## Décision (C)
- Module `external_alerts` du serveur, lancé par le rôle `web` (c'est lui qui sert la page et le WebSocket de la pastille ; le hub est en mémoire de processus). Prometheus : `GET /api/v1/alerts`, état `firing` seulement. Grafana : `GET /api/alertmanager/grafana/api/v2/alerts?active=true&silenced=false&inhibited=false` avec `GRAFANA_USER` / `GRAFANA_PASSWORD` (déjà configurés pour le rechargement des règles). Aucun réglage nouveau.
- **Lecture seule**, décidé par l'utilisateur : pas de bouton « Acquitter » (Prometheus n'a pas cette notion), l'alerte disparaît quand sa cause disparaît.
- **Une seule pastille** : alertes d'appareil en cours + alertes Prometheus et Grafana en cours.
- Chaque source a un état : non configurée, **injoignable** (la page doit le dire plutôt qu'afficher « aucune alerte »), ou lue. Une source qui ne répond plus ne compte pour rien dans la pastille.
- Aucune notification nouvelle : l'e-mail et le SMS restent faits par le relais de l'ADR 0053 (Prometheus) et par le point de contact de Grafana. Ce module ne fait que montrer.
- Sévérité : étiquette `severity` (`info`, `warning`, `critical`), `warning` si elle manque, comme dans le relais.

## Conséquences
- Un redémarrage perd l'instantané ; il est relu aussitôt (jusqu'à une minute avant la pastille complète, le premier comptage au démarrage ne contient que les alertes d'appareil).
- Pas d'historique des alertes de la plateforme dans DomiNet : Grafana et Prometheus gardent le leur.
- Avec les rôles séparés en plusieurs processus, seul le processus `web` lit les services ; le relais d'e-mail du rôle `scheduler` continue seul de son côté (deux lectures par minute, négligeable).
