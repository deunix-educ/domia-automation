# 0039 — Relais Android : point d'accès, présence, ordres et évènements


- Statut : accepté
- Date : 2026-11-07

## Contexte
L'application Kotlin (non modifiée) ouvre `/ws/android/device/<jeton>/`, envoie `sms_received`, `call_event`, `sms_sent_ack`, comprend `auth_ok`, `send_sms`, `pong` et ignore le reste. Le schéma et le protocole sont posés (ADR 0037) ; il faut le point d'accès, la présence « en ligne », l'envoi d'ordres, et de quoi prévenir les navigateurs. Plan R5 : D2 à D6, D10.

## Décision
- **Point d'accès exact** : `/ws/android/device/{token}/`, sans cookie ni `Origin` (l'application n'en envoie pas). Jeton mal formé ou inconnu : le WebSocket s'ouvre puis se ferme avec le code **4001** (la référence le refusait avant l'acceptation ; pour l'application, qui se reconnecte après 5 s dans les deux cas, c'est identique). Un jeton mal formé ne touche pas la base. **Le jeton n'est jamais journalisé** : le chemin est masqué (`/ws/android/device/***/`) dans les journaux d'accès et les refus CSRF (`web::loggable_path`).
- **Une connexion par appareil** : une nouvelle remplace l'ancienne (fermée avec 4009). « Hors ligne » n'est posé que quand la connexion **courante** se ferme : un téléphone qui se reconnecte avant que l'ancienne ligne ne soit détectée morte ne clignote pas hors ligne.
- **Battement (D4)** : trame de contrôle toutes les 30 s ; 90 s sans aucune trame reçue (OkHttp répond seul à chaque ping) : fermeture 4408. Au démarrage du rôle web tous les appareils repassent hors ligne (la référence laissait des appareils « en ligne » après un arrêt brutal).
- **Trames reçues** : lues par `dominet_domain::android` ; JSON illisible, non-objet ou type inconnu ignorés ; trame de plus de 64 Kio : connexion coupée ; numéro vide : ignoré ; numéro, contact et corps rognés aux bornes de la base (30 / 100 / 5 000 caractères) plutôt que refusés. Un `ping` JSON reçoit `pong`.
- **Ordres** (`AndroidRelay`) : `send_sms` met une ligne dans `sms_outbox` puis pousse `send_sms {to, body, message_id}` où `message_id` est l'identifiant de cette ligne ; appareil hors ligne : refus sans rien enregistrer ; ligne partie dans le vide : marquée en échec tout de suite. `push_alert` envoie `alert_notification` à **tous** les appareils connectés (l'application actuelle l'ignore ; D8). Un accusé ne compte que pour la ligne `pending` **de son appareil** (D3).
- **Délai d'accusé (D6)** : une tâche balaie toutes les 10 s les lignes sans accusé depuis 60 s et les passe en échec (« pas d'accusé de l'appareil ») ; un accusé tardif est sans effet. Pour un destinataire de la messagerie, l'accusé (ou l'échec) met à jour son statut SMS puis l'état du message.
- **Navigateurs (D5)** : sujet `android:<id>` du WebSocket existant (`WsServerMsg::Android { device_id, event }`, événements `device_status`, `sms_received`, `call_event`, `sms_sent_ack` au format des messages de la référence). La présence est **retenue** par le hub (un abonné la reçoit tout de suite). Abonnement réservé au propriétaire ou à un administrateur ; appareil inconnu et appareil d'autrui reçoivent le même refus. Écart avec l'annexe A.2 : pas de `/ws/android/browser/<id>/` (aucun client externe).
- **Appel en cours** : pour `answered` ou un état inconnu, rien n'est écrit mais les navigateurs voient l'appel ouvert (comme la référence) ; l'appel vide que la référence diffusait quand aucun appel n'existe n'est pas rediffusé.

## Vérification contre la référence
Le consommateur de référence a été exécuté une fois sur 21 sessions de l'application ; `tests/fixtures/android/oracle.json` en garde les trames renvoyées, l'état des SMS et des appels, et les évènements diffusés ; `crates/dominet/tests/android_oracle.rs` les rejoue contre le serveur Rust et exige les mêmes résultats. Écarts connus, hors de ces sessions : appel ouvert le plus récent fermé (D10), numéro vide ignoré, **le consommateur la référence plante sur un JSON qui n'est pas un objet** (`[1]`, `42`) alors que Rust l'ignore, accusés rapprochés par la file (la référence diffusait n'importe quel `message_id`, même inconnu ; ses identifiants entraient en collision), dates sans fuseau non gérées (l'application envoie toujours un décalage).

## Conséquences
- Les pages Android (lot 4) et la messagerie (lot 5) n'ont qu'à appeler `AndroidRelay::send_sms`, lire le sujet `android:<id>` et la base.
- Un seul appareil par connexion ; si un second téléphone utilisait le même jeton, ils se déconnecteraient en boucle (chaque appareil a son jeton).
- Le délai de 60 s et le battement sont des constantes (surchargeables dans les tests par `AndroidRelay::new(Timing)`).
