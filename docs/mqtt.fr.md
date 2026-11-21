# MQTT

Client MQTT supervisé du serveur (R3). Décisions : [ADR 0005](adr/0005-mqtt-client-rumqttc.md) (choix de `rumqttc`) et [ADR 0026](adr/0026-supervised-mqtt-client.md) (comportement).

## Configuration
`MQTT_HOST` (obligatoire pour activer le client), `MQTT_PORT` (1883), `MQTT_USER`, `MQTT_PASSWORD`, `MQTT_CLIENT_ID` (`dominet`) : voir [configuration](configuration.fr.md). Le mot de passe n'apparaît jamais dans les journaux.

## Courtier (Mosquitto de production)
Un seul listener, **sans accès anonyme** ([ADR 0057](adr/0057-authenticated-mqtt.md)). DomiNet et Zigbee2MQTT utilisent le compte interne (`MQTT_USER` / `MQTT_PASSWORD`). Chaque client du LAN (Linky, SolarMax externe, Tasmota…) a son compte et ses droits : `make prod-mqtt-user NAME=linky [TOPIC=linky/#] [ACCESS=read|write|readwrite]` (mot de passe généré, affiché une fois), `make prod-mqtt-users`, `make prod-mqtt-del NAME=linky`. Pas de redémarrage : les clients connectés ne sont pas coupés. Dans le client : hôte `LAN_DOMAIN`, port 1883, utilisateur et mot de passe ; il n'écrit que sous son sujet. Un refus apparaît dans le journal de Mosquitto (`not authorised`).

**File d'attente par client** : `max_queued_messages 100000` (production et développement). Mosquitto n'en garde que 1 000 par défaut et **jette** ensuite les messages QoS 1 d'un abonné plus lent que le flux (journal : `Outgoing messages are being dropped for client …`). DomiNet ralentit volontairement sa lecture quand il est chargé : c'est cette file qui absorbe la rafale. Constat du 2026-11-13 : le test de débit perdait les derniers états sur une machine chargée.

## Lancer
- `dominet serve` : web **et** client MQTT si `MQTT_HOST` est défini. `dominet serve --only mqtt` : le client seul. `--only web` : sans MQTT.
- Journal : `MQTT connecte` / `MQTT deconnecte, reconnexion en cours` (une ligne par changement d'état, pas une par tentative).

## Comportement à connaître
- Reconnexion automatique (250 ms puis jusqu'à 5 s) ; **tous les abonnements sont repris à chaque connexion**.
- Une **publication est refusée immédiatement** si le broker est injoignable (`NotConnected`) : pas de file d'attente de commandes périmées.
- Les abonnements se changent à chaud (`MqttClient::set_subscriptions`), sans redémarrage.
- Les charges utiles sont décodées en UTF-8 (sinon Latin-1), en JSON si possible, sinon en texte.

## Code
`crates/dominet-mqtt` (`start`, `MqttClient`, `topic_matches`, `decode_payload`) ; `crates/dominet-server/src/mqtt.rs` (adaptation de la configuration, rôle).

## Tests
`cargo test -p dominet-mqtt` : unitaires, puis intégration sur le Mosquitto de dev (`make dev-up`, port 11883, `MQTT_TEST_PORT` pour en changer) et sur un Mosquitto **jetable** pour les coupures (Docker requis, image `eclipse-mosquitto:2.0`) :
aucun message accepté n'est perdu quand le broker s'arrête 3 s et revient, les publications sont refusées pendant la coupure, un broker apparu après le client est rejoint avec les abonnements déjà demandés.

## Ingestion des appareils Zigbee2MQTT (lot 3, ADR 0027)
- Le client s'abonne à `<préfixe>/#` (QoS 1). Le **préfixe** est le réglage `Z2M_BASE_TOPIC` (`zigbee2mqtt` par défaut) ; le modifier dans Réglages change l'abonnement à chaud.
- `<préfixe>/bridge/devices` : création ou mise à jour des appareils (type déduit des `exposes`, clé masquée `update`) ; `<préfixe>/bridge/info` : `permit_join` ; `<préfixe>/<nom>/availability` : disponibilité ; `<préfixe>/<nom>` : état (objet JSON) qui **remplace** l'état de l'appareil actif, met à jour `last_seen`,
  alimente l'historique des clés suivies et est **diffusé en direct** (sujet `device:<id>`, dernier état retenu). Les sujets `bridge/…`, `…/set`, `…/get` et les charges non JSON ou non objet sont ignorés.
- `<préfixe>/bridge/logging` : **erreurs signalées par Zigbee2MQTT** (niveau `error` seulement). Une commande publiée réussit toujours du point de vue du broker ; si Zigbee2MQTT n'arrive pas à la remettre (appareil injoignable, erreur ZCL), il le dit ici : `Publish 'set' 'state' to 'Volet-salon' failed…`. DomiNet écrit la ligne dans son journal (avertissement « erreur signalee par Zigbee2MQTT », visible dans `/admin/logs` et comptée par `make prod-report`) et affiche un **bandeau** aux comptes connectés ; la même erreur n'est pas réaffichée avant 60 s. L'automation ou la scène à l'origine de la commande reste comptée réussie : l'erreur arrive après coup et Zigbee2MQTT ne dit pas de quelle commande elle vient.
- Au démarrage, le dernier état connu de chaque appareil actif est republié vers les widgets.
- Les messages qui s'accumulent (rafale) sont traités par paquets : l'état d'un appareil n'est écrit qu'une fois par seconde au plus dans un paquet (le dernier gagne).

### Rejeu de sessions enregistrées
`tests/fixtures/z2m/session-r3.jsonl` est une session de messages ; son résultat attendu `session-r3.golden.json` est un fichier de référence figé. Les tests Rust (`cargo test -p dominet-automation --test replay`) rejouent la même session et exigent le même résultat — à un écart près, documenté dans l'ADR 0027 (métadonnées écrites dès la création).
Pour ajouter un cas : compléter le fichier `.jsonl` et le résultat attendu du fichier `.golden.json`, en le vérifiant à la main.

## Sources MQTT, historique et archive brute (lot 4, ADR 0028)
- **Sources** : page d'administration `/admin/mqtt-sources` (administrateurs). Chaque source active ajoute son **filtre** (`linky/#`, `solarmax/+/prod`…) et son QoS aux abonnements ; l'ajout, la modification ou la suppression sont pris en compte **à chaud**, sans redémarrage
  (un rechargement de sécurité a lieu toutes les 60 s).
- Un message hors du préfixe Zigbee2MQTT dont le topic correspond à une source : **journal** si « Journaliser les messages » (topic et début de la charge utile) ; **appareil automatique** si la case est cochée et que la charge est un objet JSON (nom = topic sans le préfixe de la source, type choisi, état remplacé,
  appareil réactivé s'il était désactivé, diffusé en direct, historisé) ; les appareils dont `préfixe/nom` est ce topic sont marqués vus. Le **code de traitement** de la source est un script Rhai exécuté à chaque message (voir [scripts.md](scripts.fr.md)) ; un code Python est refusé à l'enregistrement.
- **Cocher « Appareil automatique » dès la création de la source.** Un appareil qui publie un message **retenu** (le `report` du Linky, par exemple) ne l'envoie qu'à l'abonnement : si la case est cochée plus tard, le message est déjà passé et l'appareil n'apparaît qu'au prochain message ou au prochain réabonnement (redémarrage de `dominet`, reconnexion au broker).
- **Historique** : PostgreSQL (`device_state_log`, une ligne par clé suivie et par seconde) alimente les graphiques ; purge quotidienne au-delà de `HISTORY_RETENTION_DAYS` (400 jours par défaut).
- **Archive brute** : si `REDUCTSTORE_URL` est renseigné, chaque message qui écrit une ligne d'historique ajoute un enregistrement au bucket `iot` (entrée `dev_<id>`, corps = état complet, une étiquette par clé suivie numérique), quota FIFO `IOT_QUOTA_GB` (5 Go par défaut, appliqué à la création du bucket).
  Elle ne ralentit jamais l'ingestion : file bornée, écriture par lots de 50 en tâche de fond ; ReductStore injoignable → les enregistrements sont abandonnés (avertissement dans le journal) et l'écriture reprend d'elle-même.

| Variable | Défaut | Rôle |
|---|---|---|
| `HISTORY_RETENTION_DAYS` | `400` | âge maximal des lignes de `device_state_log` |
| `IOT_QUOTA_GB` | `5` | quota FIFO du bucket `iot` (à la création) |
| `REDUCTSTORE_IOT_BUCKET` | `iot` | nom du bucket (essais sur une stack partagée) |

Tests : `cargo test -p dominet-automation --test sources` (sources et appareils automatiques), `-p dominet-store --test reduct` et `-p dominet-server --test mqtt_role` (ReductStore et broker de dev), E2E `mqtt` (archive) et `mqtt_sources` (page d'administration).

### Rejeu des sources
`tests/fixtures/mqtt/sources-r3.jsonl` (+ `.golden.json`) rejoue des **sources MQTT** (appareils automatiques, journal, marquage « vu », réactivation, sources inactives) : `cargo test -p dominet-automation --test replay_sources`. Un seul écart, voulu : le premier message d'un appareil automatique n'en écrit pas l'état (ADR 0028).
Débit : `cargo test --release -p dominet-server --test throughput -- --nocapture`.

