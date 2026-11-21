# ADR 0057 — MQTT non anonyme : un listener, des comptes nominatifs, des droits par sujet


Statut : accepté (R8 lot 2 ; décision de l'utilisateur du 2026-11-10, qui tranche D7 de R7 ; remplace la partie « listener LAN anonyme » de l'ADR 0055).

## Contexte
L'ADR 0055 gardait un second listener (1884) **anonyme** pour les appareils existants, désactivable par `MQTT_LAN_ANONYMOUS`, la décision finale étant renvoyée à R8. La pile Rust s'installe à neuf, sans client MQTT existant à ménager (SolarMax est lu par le rôle `publishers` de DomiNet, qui publie en interne ; Linky publiera plus tard).

## Options
1. Garder le listener anonyme, désactivé par défaut : un oubli de configuration réouvre le broker au LAN.
2. Un seul compte partagé par tous les clients externes : simple, mais aucune séparation entre clients.
3. **Un seul listener, mot de passe obligatoire, un compte par client et des droits par sujet** (retenu).

## Décision
- **Un seul listener, 1883**, `allow_anonymous false`, publié sur le port `MQTT_LAN_PORT` de l'hôte (1883 par défaut). Plus de 1884, plus de `MQTT_LAN_ANONYMOUS`. Les services de la pile (DomiNet et Zigbee2MQTT) utilisent le compte interne `MQTT_USER` / `MQTT_PASSWORD` (droit `readwrite #`) : même zone de confiance, le réseau Docker.
- **Comptes externes** dans le volume `mosquitto-accounts` : `make prod-mqtt-user NAME=linky [TOPIC=linky/#] [ACCESS=read|write|readwrite]` crée le compte (mot de passe généré et affiché **une fois**, haché pbkdf2 dans le volume), `prod-mqtt-del`, `prod-mqtt-users`. Le sujet par défaut est `NOM/#` : un client ne voit et n'écrit que son sujet.
- `deploy/mosquitto/accounts.sh` (monté dans le conteneur) régénère `passwd` et `acl` à chaque démarrage et à chaque ajout ou retrait, puis envoie `SIGHUP` à Mosquitto : **aucun redémarrage**, les clients connectés ne sont pas coupés.
- Noms limités à `[a-z0-9_-]`, 32 caractères, jamais celui du compte interne ; sujets limités à des caractères MQTT sûrs.
- La pile de développement garde son broker local anonyme (`mosquitto.dev.conf`, boucle locale uniquement) : les tests n'ont pas à gérer de secret.

## Conséquences
- Tout client du LAN doit recevoir un compte **avant** de publier ; un refus se voit dans le journal de Mosquitto (`Connection refused: not authorised`).
- Les appareils MQTT en WiFi (Tasmota, ESP…) se configurent avec l'utilisateur, le mot de passe et le port ; les capteurs qui ne savent pas s'authentifier ne sont pas pris en charge sur ce broker.
- Pas de TLS sur MQTT dans ce lot : le mot de passe circule en clair sur le LAN ; TLS (8883) est une amélioration possible.
- Contrôlé par `tools/check-stack.sh` (refus de l'anonyme sur 1883 depuis le réseau Docker et depuis l'hôte, 1884 fermé, compte externe : publication dans son sujet, refus ailleurs, mauvais mot de passe, suppression) et `tools/check-install.sh` (cibles `make prod-mqtt-*`).
