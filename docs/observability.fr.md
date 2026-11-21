# Observabilité : métriques, alertes, tableaux Grafana

DomiNet expose ses métriques à Prometheus, relaie ses alertes par e-mail et SMS, et fournit les tableaux Grafana de la plateforme. Mise en place de bout en bout : voir le déploiement (R7 lot 5).

## Métriques (`/metrics`)
Un écouteur à part (`METRICS_ADDR`, défaut `0.0.0.0:9100`, `off` pour le couper), **jamais publié** sur l'hôte : seul Prometheus le joint, par le réseau Docker. Le port public répond 404 sur `/metrics`. Signaux : requêtes HTTP (méthode, route, statut, latence), WebSocket, MQTT, tâches planifiées (résultat, durée), segments d'enregistrement, mouvements et détections, pool PostgreSQL, alertes Prometheus en cours, version. Les étiquettes sont bornées (route normalisée, jamais l'URL brute) et ne contiennent aucun secret. [ADR 0051](adr/0051-prometheus-metrics.md).

## Règles d'alerte
Éditées dans `/admin/prometheus-rules` et `/admin/grafana-rules`, écrites en fichiers de provisionnement, services rechargés : voir [règles Prometheus et Grafana](prometheus-grafana-rules.fr.md).

## Alertes de la plateforme dans l'interface
La page [alertes](alerts.fr.md) montre aussi, en lecture seule, les alertes en cours de Prometheus et de Grafana (lues chaque minute, [ADR 0061](adr/0061-platform-alerts-in-the-alerts-page.md)) et la pastille les compte. C'est un affichage : l'envoi par e-mail et SMS reste fait par le relais ci-dessous (Prometheus) et par le point de contact de Grafana.

## Relais des alertes Prometheus
Avec `PROMETHEUS_URL`, le rôle `scheduler` lit `/api/v1/alerts` chaque minute. Une alerte **nouvelle** en état `firing` envoie un e-mail « [DomiNet] Alerte » (destinataire des alertes) et, si elle est `critical`, un SMS Free Mobile ; elle se tait ensuite 1 h (`PROMETHEUS_ALERT_COOLDOWN`). Quand elle **disparaît**, un e-mail « RESOLU » part (jamais de SMS). Chaque alerte a sa propre identité (une clé unique pour toutes ferait taire la deuxième alerte), et le message « RESOLU » part réellement. [ADR 0053](adr/0053-prometheus-alerts-and-grafana-dashboards.md).

## Tableaux Grafana
Sept tableaux génériques dans `deploy/grafana/provisioning/dashboards/` : vue générale (HTTP, tâches, base), automation et appareils, surveillance (MediaMTX), détail d'une caméra (variable `camera_uuid`), météo (variable `station`), historique d'un appareil (variables `device_id` et `key`) et **énergie** (voir ci-dessous). Les six premiers sont des fichiers JSON du dépôt ; l'énergie est écrite à la main. Les tableaux **d'un appareil** sont produits par l'application (option « tableau Grafana » de l'appareil). Sources de données `postgresql` et `prometheus`, point de contact `dominet-email` (e-mail à `EMAIL_RECIPIENT`) et politique de notification dans `deploy/grafana/provisioning/`.

## Tableau « DomiNet — Énergie »
Un tableau (`energy.json`, 30 jours par défaut) qui reprend le bilan de la page [énergie](energy.fr.md) : production du jour (clé `qdy`), puissances en direct (production `pac`, Linky `PAPP`, pompe à chaleur `power_ab`), **bilan net en direct** (production − Linky − PAC), histogramme par jour (solaire, Linky, PAC, bilan net en courbe, lu dans `energy_daily_log`) et **autoconsommation**.

- **Autoconsommation** = part de la production solaire consommée par la maison elle-même : `min(production, Linky + PAC)` divisé par la production, en %. Un jour sans production n'a pas de taux (et non 0 %). Le panneau « sur la période » somme les jours avant de diviser (il ne fait pas la moyenne des taux).
- Les appareils sont retrouvés par leurs clés d'état (`pac`, `PAPP`, `power_ab`, `qdy`), pas par leur nom : les réglages `ENERGY_*_DEVICE` n'ont pas à être recopiés dans Grafana.
- Le panneau « Puissances en direct » demande que `pac`, `PAPP` et `power_ab` soient **suivies** dans l'historique de leurs appareils (page Appareils, clés suivies) ; `dominet seed-energy` les ajoute avec les compteurs du bilan journalier. Sans elles le panneau reste vide, le reste du tableau marche.

## Contrôles
- `cargo test -p dominet-server --test grafana_assets` : chaque requête SQL des tableaux s'exécute sur le schéma de DomiNet, chaque métrique citée existe (DomiNet ou MediaMTX 1.21.3), et le taux d'autoconsommation est calculé juste sur des jours connus.
- `tools/check-observability.sh` : vrais Prometheus et Grafana jetables (ports décalés, base jetable) chargent règles, tableaux (sept), sources, points de contact et politiques sans erreur, exécutent les requêtes du tableau énergie, et acceptent les rechargements demandés par DomiNet.
