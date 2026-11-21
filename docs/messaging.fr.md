# Messagerie interne

R5 lot 5, [ADR 0041](adr/0041-asynchronous-messaging.md) (modèle : [ADR 0037](adr/0037-android-relay-and-messaging.md), canaux : [ADR 0038](adr/0038-delivery-channels.md), SMS : [ADR 0039](adr/0039-android-relay-server.md)).

Un administrateur écrit un message à des comptes ; il part par e-mail, par SMS (via un téléphone Android connecté) ou les deux.

## Pages

| Adresse | Qui | Contenu |
|---|---|---|
| `/messaging/compose` | administrateurs | sujet, message, canal (e-mail / SMS / les deux), destinataires : tous les comptes actifs, les administrateurs, ou des comptes choisis |
| `/messaging` | administrateurs | les 100 derniers messages envoyés |
| `/messaging/inbox` | tout compte | les 100 derniers messages reçus, avec son propre état e-mail / SMS |
| `/messaging/<id>` | expéditeur, destinataires, administrateurs | le message ; l'expéditeur et les administrateurs voient le **tableau de livraison** complet, un destinataire seulement son état |

## Comment un message part

« Envoyer » enregistre le message et ouvre aussitôt sa page ; l'envoi se fait ensuite en tâche de fond et **les statuts changent en direct** :

1. **E-mail** : un message par destinataire qui a une adresse (sujet = sujet du message, texte brut). Serveur SMTP : variables `EMAIL_*` ; expéditeur : réglage `DEFAULT_FROM_EMAIL`.
2. **SMS** : confié à l'appareil Android **en ligne le plus récemment vu**. Le statut reste « en attente » jusqu'à l'accusé de l'application ; **sans accusé au bout de 60 s, il passe en échec**.
3. Le message est « terminé » quand plus rien n'est en attente. « Terminé » ne veut pas dire « tout a réussi » : le tableau montre chaque échec avec sa raison (`email: …`, `sms: aucun appareil Android connecte`, `sms: pas d'accuse de l'appareil`…).

Un destinataire sans adresse (ou sans mobile) pour le canal choisi est « sans objet » pour ce canal. Le formulaire signale ces comptes et prévient quand aucun serveur d'e-mail n'est configuré ou qu'aucun téléphone n'est en ligne.

## Droits et vie privée

- Composer et lister les messages envoyés : administrateurs seulement.
- Un message qui n'existe pas et un message qui ne vous concerne pas reçoivent la même réponse : « introuvable ».
- Suivi en direct : sujet `message:<id>` du WebSocket, mêmes droits que la page.

## Après un arrêt du serveur

Un message resté « en cours » à cause d'un arrêt brutal est clos au redémarrage : ses envois en attente sont marqués en échec (« interrompu par un redémarrage du serveur »). Rien ne repart tout seul ; l'expéditeur peut renvoyer.

## Comportements notables

- Envoi **asynchrone** (la requête n'attend pas).
- SMS : **délai de 60 s**.
- Destinataire sans adresse ou sans mobile : « sans objet ».
- Listes de 100 lignes au lieu de pages de 20 ; pas de brouillons.
