# 0019 — Protocole WebSocket multiplexé : sujets typés, dernier état conservé


- Statut : accepté
- Date : 2026-11-01

## Contexte
R1 a livré un hub WebSocket de base (ADR 0016) : une connexion par onglet, un seul sujet (`notifications`), des messages diffusés à tous. R2 doit y faire
passer l'état des appareils (puis les statistiques système, les caméras…) : le tableau de bord suit des dizaines d'appareils, chacun n'intéressant que les
onglets qui l'affichent. Il faut aussi qu'un onglet qui s'abonne reçoive tout de suite l'état courant, sans attendre le prochain message de l'appareil
(qui peut ne jamais venir pour un capteur discret).

## Options
**Connexions.** (a) Une connexion WebSocket par sujet — multiplie les connexions (limite de 8 par compte) et les battements. (b) **Une connexion
multiplexée**, le client s'abonne et se désabonne par message (décision D3 du plan R2) — retenue.

**Sujets.** (a) Chaînes libres — une faute de frappe devient un sujet fantôme. (b) **Type `Topic`** dans `dominet-proto` (`Notifications`, `System`,
`Device(i64)`, `Camera(Uuid)`), avec une forme textuelle canonique stricte (`device:7`, jamais `device:07` ni `device:+7`) — retenue : le serveur et le
navigateur partagent le même type, un sujet invalide est refusé à l'analyse.

**État initial.** (a) Le client interroge une API REST à l'abonnement — deux chemins à garder cohérents, et une fenêtre de course entre la réponse et
le premier message. (b) **Le hub garde le dernier message de chaque sujet** (« retained », comme MQTT) et l'envoie après `Subscribed` — retenue.

## Décision
- Messages client → serveur : `subscribe {topic}`, `unsubscribe {topic}`. Serveur → client : `subscribed {topic}`, `refused {topic, reason}` avec
  `reason` ∈ `invalid` (sujet illisible), `forbidden` (droit insuffisant), `limit` (128 abonnements par connexion), puis les messages du sujet
  (`device_state`, `notification`, …), au format plat `{"type": …}`.
- **Autorisation par sujet** (`authorize` dans `web/ws.rs`) : tout compte connecté lit `notifications`, `system` et `device:*` ; `camera:*` est **refusé**
  en R2 (aucun flux avant R6, le droit sera décidé avec la table des caméras). L'abonnement à `notifications` est automatique et ne peut pas être retiré.
- **Dernier état** : `Hub::publish_retained` enregistre le message *avant* de le diffuser, de sorte qu'un abonné qui arrive pendant la publication reçoit le
  snapshot ou le message, jamais ni l'un ni l'autre. Mémoire bornée en R2 par le nombre d'appareils ; R3 regroupera les rafales par appareil (point
  d'extension déjà prévu, plan R2 « Débit R3 »).
- **Côté serveur**, chaque connexion tient ses abonnements dans une table ; chacun est une tâche qui relaie le canal du sujet vers la boîte de la
  connexion et qui est **interrompue à la fermeture** (test : les abonnements sont libérés). S'abonner deux fois est sans effet (idempotent) ; un message
  client inconnu ou mal formé est ignoré, plafonné à 1 Kio.
- **Côté navigateur**, les composants s'abonnent par `Realtime::subscribe(&Topic)` / `unsubscribe`, qui **compte les références** : deux widgets sur le
  même appareil ne produisent qu'un abonnement, retiré quand le dernier disparaît. Un effet (`layout.rs`) envoie la différence entre l'état voulu et l'état
  envoyé (`subscription_diff`). À chaque (re)connexion, à réception de `hello`, le client oublie ce qu'il croyait abonné et renvoie tout : la
  reconnexion après redémarrage du serveur est donc transparente (E2E).
- **Outil de simulation** (Réglages → Simulateur d'état d'appareil, administrateurs) : fonction serveur `inject_device_state` qui publie un état sous
  `device:<id>` (objet JSON de 4 Ko au plus). Il sert de premier consommateur réel du protocole avant l'arrivée de MQTT (R3) et de banc d'essai des widgets du
  lot 4 ; il disparaîtra ou deviendra un outil de diagnostic une fois R3 livrée.

## Conséquences
- Le tableau de bord (lots 4 et 5) n'a qu'à s'abonner aux appareils qu'il affiche ; R3 publiera avec `publish_retained` sans toucher au protocole.
- Un sujet de plus = une variante de `Topic` (+ son `Display`/`FromStr`, ses tests de forme canonique) et une règle dans `authorize`.
- L'accès aux flux caméra devra être décidé en R6 (aujourd'hui tout refus) ; le droit de lecture des appareils par tous les comptes est une décision de R2 à
  revoir si des droits par appareil apparaissent.
- Le dernier état reste en mémoire : un redémarrage du serveur le vide jusqu'au prochain message (R3 le rechargera depuis la base).
- `WsServerMsg` n'est plus `Eq` (il porte un `serde_json::Value`) ; `PartialEq` suffit aux tests.
