# Relais Android (côté serveur)

R5 lot 3, [ADR 0039](adr/0039-android-relay-server.md) (modèle : [ADR 0037](adr/0037-android-relay-and-messaging.md)). L'application Android n'est **pas modifiée** : le serveur Rust parle exactement son protocole.

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
