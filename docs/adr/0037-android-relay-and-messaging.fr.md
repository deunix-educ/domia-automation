# 0037 — Relais Android et messagerie : modèle de données


- Statut : accepté
- Date : 2026-11-06

## Contexte
R5 reprend le relais Android (un téléphone envoie SMS reçus et événements d'appel, reçoit des SMS à émettre) et la messagerie interne (un message, plusieurs destinataires, e-mail et/ou SMS). Les tables `message`, `message_recipient`, `android_device`, `sms_message`, `call_log` existent depuis R1. L'application Kotlin ne doit pas changer (plan R5, D1 à D12 ; protocole lu dans `CommWebSocket.kt`).

## Options
- **Jeton en clair** (comme la référence) : une fuite de la base donne accès à tous les appareils.
- **Empreinte du jeton** : seule l'empreinte SHA-256 est stockée ; le clair n'est montré qu'à la création ou à la régénération.
- **Accusés de SMS** : réutiliser `sms_message.id` (SMS du navigateur) et `message_recipient.id` (messagerie) comme `message_id` du protocole — ambigu, deux espaces d'identifiants.

## Décision
- **Migration 0010** : `android_device.token` (UUID) est remplacé par `token_hash` (SHA-256 hexadécimal, 64 caractères, unique). Le jeton est produit par `gen_random_uuid()` **en SQL**, l'empreinte aussi : le clair ne transite que dans la réponse de la création / régénération. Les appareils existants sont migrés (l'empreinte du jeton actuel : les téléphones déjà configurés continuent de fonctionner). Un jeton mal formé dans l'adresse (`canonical_token`) est traité comme inconnu.
- **File `sms_outbox`** : un SMS demandé à un appareil a son propre identifiant, qui devient le `message_id` de `send_sms` et revient dans `sms_sent_ack`. L'origine est exactement l'une de : un SMS du navigateur (`sms_message_id`) ou un destinataire de la messagerie (`message_recipient_id`) (contrainte `sms_outbox_one_origin`). Un accusé ne modifie que les lignes `pending` **de l'appareil qui l'envoie** : un autre appareil, un identifiant inconnu ou un second accusé sont sans effet. Les lignes `pending` plus vieilles que le délai (60 s) passent en `failed` (`expire`).
- **Journal d'appels** (`plan_call`, pur) : `ringing` / `outgoing` créent ; `missed` / `ended` ferment l'appel ouvert du même numéro, ou créent une entrée déjà terminée ; `answered` et les états inconnus ne font rien. **Écart voulu avec la référence** : l'appel ouvert le plus **récent** est fermé (la référence prenait le plus ancien).
- **Messagerie** : états `pending` / `sent` / `failed` / `na` par canal et destinataire (`initial_statuses` : `pending` seulement si le canal est demandé et la coordonnée existe) ; le message passe à `sent` quand plus aucun envoi n'est en attente (`finish_if_complete`, atomique).
- **Protocole** dans `dominet_domain::android` (pur, testé contre des trames tirées du code Kotlin, `tests/fixtures/android/frames.json`) : `parse_device_frame` applique les valeurs par défaut de la référence ; un JSON illisible est ignoré.
- **Ports** : `AndroidDeviceRepo`, `SmsRepo`, `CallRepo`, `SmsOutboxRepo`, `MessageRepo` ; adaptateurs `PgAndroidDevices`, `PgSms`, `PgCalls`, `PgSmsOutbox`, `PgMessages`.

## Conséquences
- La migration est à sens unique : le jeton en clair n'existe plus nulle part ; le perdre impose de le régénérer (et de reconfigurer le téléphone).
- À la bascule (R8), l'import d'appareils existants devra recalculer l'empreinte à partir du jeton existant (`encode(sha256(convert_to(token::text,'UTF8')),'hex')`).
- Les lots suivants ajoutent le point d'accès WebSocket, la messagerie asynchrone et les canaux réels (SMTP, Free Mobile) sur ces ports.
