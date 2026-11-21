# 0026 — Client MQTT supervisé : publications immédiates ou refusées, resouscription à chaque connexion


- Statut : accepté
- Date : 2026-11-02

## Contexte
R3 écoute Zigbee2MQTT et les sources MQTT, et publie les commandes des appareils, les mesures météo et SolarMax. Le spike 8 (ADR 0005) a validé `rumqttc`. Il reste à décider le comportement du client dans la durée :
que faire d'une commande quand le broker est injoignable, comment reprendre après une coupure, comment changer les abonnements à chaud, comment arrêter proprement.

## Décisions
- **Crate `dominet-mqtt`**, sans dépendance vers le serveur : réglages (`MqttSettings`, mot de passe masqué dans `Debug`), client (`start`), décodage des charges utiles, correspondance de topics. Le serveur adapte sa configuration (`mqtt::settings`).
  `rumqttc` sans fonctionnalité par défaut : TCP en clair (broker du réseau local, anonyme ou à mot de passe) ; TLS sera ajouté quand un broker distant le demandera.
- **Une tâche supervise la connexion** : `poll()` en boucle, reprise après erreur avec délai croissant (250 ms à 5 s), **resouscription de tout l'ensemble voulu à chaque `ConnAck`** (on ne suppose rien de la session du broker, ADR 0005), état exposé par un canal `watch`
  (`Connecting`, `Connected`, `Disconnected`). Session persistante (`clean_session = false`) et identifiant de client stable.
- **Une publication échoue tout de suite si le lien est coupé** (`PublishError::NotConnected`) au lieu d'être mise en file. Une commande de lumière ou de volet exécutée dix minutes après le clic est pire qu'une erreur affichée. Un message accepté
  par l'API est livré **au moins une fois** (QoS 1, renvoi après reconnexion) ; ce que le broker avait accepté sans le transmettre ne survit à son arrêt que s'il persiste ses données (celui de production le fait).
- **Abonnements voulus** : un ensemble `filtre → QoS` remplacé d'un bloc (`set_subscriptions`) ; la tâche applique la différence tout de suite si le lien est établi (`diff_subscriptions`, pure et testée), sinon à la prochaine connexion. Un changement de QoS se resouscrit. Les sources MQTT modifiées dans l'interface
  s'appliqueront ainsi sans redémarrage (lot 4).
- **Messages reçus décodés** comme dans la référence : UTF-8 (sinon Latin-1), JSON si possible, sinon texte. File d'entrée de 8192 messages : quand elle est pleine, la lecture du réseau attend (le broker ralentit, rien n'est perdu).
- **Arrêt** : `shutdown()` (niveau, pas évènement : une pause en cours ne l'absorbe pas), déconnexion propre, fin de la tâche. Si plus personne ne consomme les messages ou ne tient la poignée, la tâche s'arrête.
- **Rôles** : `dominet serve` lance `web` et, si `MQTT_HOST` est renseigné, `mqtt` ; `--only mqtt` fait tourner le client seul (le processus attend alors le signal d'arrêt). Identifiant par défaut `dominet` (`MQTT_CLIENT_ID`), différent de celui de l'implémentation de référence pour que les deux coexistent sur un broker pendant la migration.
- **Tests** : unitaires (correspondance, décodage, différence d'abonnements, topics de publication, mot de passe masqué) ; intégration sur le Mosquitto de dev (aller-retour, messages retenus, abonnements à chaud, arrêt) et sur un **Mosquitto jetable** (conteneur nommé, persistant, supprimé à la fin) pour les coupures : le broker partagé n'est jamais arrêté.

## Conséquences
- Pendant une coupure, les commandes sont refusées et les mesures de publishers (lot 6) sont perdues pour le cycle en cours ; elles reprennent au cycle suivant. Les états des appareils, eux, viennent du broker (messages retenus de Zigbee2MQTT) à la reconnexion.
- Les tests de coupure exigent Docker (déjà requis par la stack de dev) et l'image `eclipse-mosquitto:2.0`.
- Les messages QoS 1 peuvent arriver en double : l'ingestion (lot 3) est idempotente (ADR 0025).
