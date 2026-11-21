# 0005 — Client MQTT : rumqttc (spike 8)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO**

## Contexte
Le bus d'intégration est MQTT (Z2M, Linky, SolarMax, météo). Il faut un client Rust fiable pour
`dominet-mqtt` : reconnexion automatique, QoS 1, jokers `+`/`#`, débit largement supérieur au réel
(≈ 50 devices à 1 Hz, soit quelques dizaines de msg/s). Code du spike : `spike (code non conservé)`
(`rumqttc` 0.25.1, Mosquitto 2.0 de dev, EliteDesk i5-6500).

## Mesures
| Test | Résultat |
|---|---|
| Topic matching, 19 cas (`zigbee2mqtt/#`, `+/availability`, `solarmax/+/+`, `$SYS`, niveaux vides…) | implémentation maison : 19/19 ; `rumqttc::matches` diverge sur `$SYS/#` (sans conséquence ici) |
| 50 devices × 1 Hz, QoS 1, 10 s | 500/500, 0 perte |
| 2000 msg/s, QoS 1, 10 s | 20 000/20 000, 0 perte |
| Rafale maximale QoS 1 (43 000 msg/s émis) | 49 251/50 000 reçus (749 perdus), 33 800 msg/s reçus |
| Rafale maximale QoS 0 (285 000 msg/s émis) | 50 000/50 000 reçus |
| Coupure du broker 5 s pendant 100 msg/s QoS 1 (sessions persistantes) | 3000 émis, **0 manquant**, 1 doublon ; reconnexion 0,6 s après le retour du broker ; resouscription à chaque `ConnAck` |

## Décision
- `dominet-mqtt` s'appuie sur **`rumqttc`** (QoS 1 par défaut, `clean_session = false`, identifiant de client stable).
- Boucle d'événements supervisée : sur erreur, attente de 250 ms puis nouveau `poll()` (reconnexion automatique) ; **resouscription à chaque `ConnAck`** pour ne pas dépendre de la session côté broker.
- `topic_matches` est implémenté dans `dominet-mqtt` (code du spike repris), avec sa table de cas en tests unitaires ; on n'utilise pas `rumqttc::matches`.
- Traiter les doublons QoS 1 : les consommateurs (état device, historique) doivent être idempotents (déduplication à la seconde déjà prévue pour `device_state_log`).

## Conséquences
- À forte rafale (≫ 1000 msg/s) le broker peut abandonner des messages QoS 1 (file par client de Mosquitto) : sans objet au débit réel, à surveiller dans les tests de charge de R3/R8.
- Pièges notés : détecter l'inactivité sur les `Publish` reçus, pas sur `poll()` (les `PingResp` relancent un timeout naïf) ; `docker compose exec` doit recevoir `</dev/null` dans une boucle `while read`.
