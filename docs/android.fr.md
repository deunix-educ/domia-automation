# Relais Android

R5 lot 3, [ADR 0039](adr/0039-android-relay-server.md) (modèle : [ADR 0037](adr/0037-android-relay-and-messaging.md)). Le serveur Rust parle exactement le protocole de l'application. L'application elle-même est dans ce dépôt (`android/`), voir [L'application](#lapplication-domocomm) plus bas ([ADR 0066](adr/0066-android-application.md)).

## Ce que fait l'application

Elle se connecte à `wss://<serveur>/ws/android/device/<jeton>/` (jeton de l'appareil, affiché une seule fois à sa création ou à sa régénération) et se reconnecte seule après 5 s.

| Sens | Trame | Effet |
|---|---|---|
| app → serveur | `sms_received {from, body, contact_name, timestamp}` | SMS enregistré (non lu), affiché en direct |
| app → serveur | `call_event {state, number, contact_name, timestamp, duration}` | journal d'appels (`ringing` / `outgoing` créent, `ended` / `missed` ferment, `answered` ne change rien) |
| app → serveur | `sms_sent_ack {message_id, success, error}` | le SMS demandé est « envoyé » ou « échoué » |
| serveur → app | `auth_ok {device_name}` | connexion acceptée ; l'appareil est « en ligne » |
| serveur → app | `send_sms {to, body, message_id}` | envoie un SMS ; l'app répond par `sms_sent_ack` avec le même `message_id` |
| serveur → app | `pong`, `alert_notification {…}` | (l'application actuelle ignore ce dernier) |

Jeton inconnu : fermeture avec le code **4001**.

## Présence et battement

Le serveur envoie une trame de contrôle toutes les 30 s ; sans rien recevoir pendant 90 s il ferme (code 4408). Une nouvelle connexion du même appareil remplace l'ancienne (4009). Au démarrage tous les appareils repassent « hors ligne ».

## SMS à émettre

`AndroidRelay::send_sms` met le SMS en file (`sms_outbox`) puis l'envoie à l'appareil ; l'accusé le règle. **Sans accusé au bout de 60 s, le SMS passe en échec** (« pas d'accusé de l'appareil »). Un accusé d'un autre appareil, d'un identifiant inconnu ou tardif est sans effet.

## Pour les navigateurs

Sujet `android:<id>` du WebSocket des notifications (propriétaire ou administrateur) : `device_status`, `sms_received`, `call_event`, `sms_sent_ack`. La présence courante arrive dès l'abonnement.

## Les pages (lot 4, [ADR 0040](adr/0040-android-pages.md))

| Page | Contenu |
|---|---|
| `/android` | appareils (présence en direct), 10 derniers SMS et appels, nombre de SMS non lus |
| `/android/devices` | ajouter un appareil (nom, numéro), **nouveau jeton**, supprimer (avec confirmation) ; **le jeton n'est montré qu'à la création ou à la régénération** |
| `/android/sms?device=N` | les 100 derniers SMS (les reçus sont marqués lus à l'affichage), envoi d'un SMS ; chaque SMS sortant montre son état : en attente, envoyé, échec et sa raison |
| `/android/calls?device=N` | les 100 derniers appels (entrant, sortant, manqué, durée `mm:ss`, « en cours ») |

Chaque compte voit ses appareils, un administrateur voit tous les appareils. Tout se met à jour en direct (SMS reçus, appels, accusés, présence). Adresse à saisir dans l'application : `ws://adresse-du-serveur/ws/android/device/JETON/` (`wss://` derrière HTTPS).

## Sécurité

- Le jeton n'est stocké que haché (SHA-256) et **n'apparaît dans aucun journal** (le chemin est masqué).
- Trames limitées à 64 Kio ; champs rognés aux bornes de la base ; JSON invalide ignoré sans fermer la ligne.

## Tests

`cargo test -p dominet --test android_ws` (15 : jeton, présence, remplacement, battement, SMS, appels, accusés, messagerie, délai, alertes, droits d'abonnement), `--test android_oracle` (**21 sessions rejouées contre des échanges de référence figés**), `-p dominet-domain --test android_fixtures` (trames du code Kotlin), `--test ui android` (droits, jeton, envoi), E2E `android` (téléphone simulé `tools/sim_android.py`).

## L'application (DomoComm)

DomoComm est l'application Android qui relaie les SMS et les appels d'un téléphone vers DomiNet. Son code est dans `android/` (Kotlin, package `com.domoconf.comm`, même licence que le reste). Il faut un téléphone avec une carte SIM et Android 8.0 ou plus.

### L'installer

1. Téléchargez `DomoComm-<version>.apk` sur la page de release GitHub du projet (étiquettes `android-<version>`). Vérifiez-le avant l'installation : la note de release donne son SHA-256 (`sha256sum DomoComm-<version>.apk`) et l'empreinte du certificat de signature, identique pour toutes les versions :
   `77:AE:DB:BD:B3:ED:C7:A9:90:E6:32:29:0C:A8:63:BA:43:55:B3:C9:C3:6C:A9:86:A0:DE:7B:5A:E8:1E:EB:F2`
2. Ouvrez le fichier sur le téléphone et autorisez une fois votre navigateur ou votre gestionnaire de fichiers à installer des applications. Une mise à jour ultérieure garde la configuration, tant que l'APK est signé avec la même clé.
3. **Android 13 et plus : autorisez les paramètres restreints.** Une application installée depuis un fichier ne peut pas recevoir les permissions SMS et appels tant que vous ne l'avez pas autorisé : *Paramètres, Applications, DomoComm, menu (trois points), Autoriser les paramètres restreints*. Le menu apparaît après un premier refus de permission.
4. Ouvrez DomoComm, touchez **+**, donnez un nom et l'URL de l'appareil (ci-dessous), puis **Démarrer** et acceptez les permissions.

### L'URL

Sur la page **Android, Appareils** de DomiNet, créez un appareil : son jeton n'est affiché qu'**une fois**. L'URL à saisir est `wss://<serveur>/ws/android/device/<jeton>/` (`<serveur>` est le nom ou l'adresse avec lesquels vous ouvrez DomiNet, `dominet.local` ou l'adresse du VPN). Un `ws://` simple est refusé par Android, sauf si le serveur est joint sans TLS.

### Faire confiance au certificat de votre serveur

DomoComm fait confiance aux autorités d'Android et à **celles que vous avez installées sur le téléphone** ; il n'en embarque aucune. Avec l'autorité locale créée par `make certs` (mkcert), téléchargez `https://<serveur>/dominet-ca.crt` et installez-la : *Paramètres, Sécurité, Chiffrement et identifiants, Installer un certificat, Certificat d'autorité*. Le certificat du serveur doit contenir le nom ou l'adresse de l'URL (`CERT_EXTRA_SANS` pour une adresse du VPN, puis `make certs`, qui recharge aussi nginx).

### Ce que vous voyez

- La pastille à côté de l'état : verte tant que le service tourne.
- La pastille de chaque serveur : **verte tant que ce serveur est connecté**, grise sinon. Elle suit une perte et un retour du réseau.
- Si une permission est refusée, le service démarre quand même et liste ce qui manque (les permissions téléphone, journal d'appels et contacts ne limitent que les appels et les noms). Sans aucune permission SMS, il ne démarre pas.
- La notification indique combien de serveurs sont connectés.

### La compiler vous-même

`make android-apk` compile et signe `dist/DomoComm-<version>.apk` et affiche son SHA-256 et l'empreinte du certificat. Il faut un JDK 17 ou 21, le SDK Android (`ANDROID_HOME`, par défaut `~/Android/Sdk`, avec la plateforme 35 et les build-tools) et une clé de signature. Cette commande ne publie rien. `make ci` ne compile pas l'application.

Pour une version de débogage sans clé : `cd android && ./gradlew assembleDebug` (installable pour des essais, pas faite pour être distribuée).

### Clé de signature

La clé n'est **jamais dans le dépôt**. `make android-apk` lit `~/.config/domocomm/signing.properties` (ou le fichier désigné par `DOMOCOMM_SIGNING`) avec les lignes `storeFile`, `storePassword`, `keyAlias`, `keyPassword`. Pour en créer une :

```
mkdir -p ~/.config/domocomm && chmod 700 ~/.config/domocomm
keytool -genkeypair -keystore ~/.config/domocomm/domocomm-release.jks -storetype PKCS12 \
  -alias domocomm -keyalg RSA -keysize 4096 -validity 10950 -dname "CN=DomoComm, O=DomiNet"
```

Écrivez ensuite le fichier de propriétés avec le mot de passe saisi, et faites `chmod 600` sur les deux fichiers. **Sauvegardez la clé hors de la machine** : sans elle, personne ne peut mettre à jour l'application installée sur les téléphones (Android refuse une mise à jour signée par une autre clé), et une nouvelle clé oblige à installer une nouvelle application. Qui fait un fork du projet signe avec sa propre clé : son APK est, pour Android, une autre application.

### Limites

- L'application ne garde rien en dehors de la liste des serveurs (nom et URL avec jeton) dans ses préférences privées ; les sauvegardes Android sont désactivées pour que le jeton ne quitte pas le téléphone.
- L'envoi d'un SMS passe par le gestionnaire de SMS par défaut du téléphone : les règles et les limites de l'opérateur s'appliquent.
