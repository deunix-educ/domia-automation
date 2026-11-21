# 0041 — Messagerie : envoi asynchrone, statuts en direct, droits


- Statut : accepté
- Date : 2026-11-07

## Contexte
La messagerie interne (un message, plusieurs comptes, e-mail et/ou SMS) existe dans la référence avec un envoi **synchrone** : la requête attend chaque e-mail, et un SMS reste « en attente » sans délai maximal. Les canaux (ADR 0038), le relais Android (ADR 0039) et le modèle de données (ADR 0037) existent. Plan R5 : D6, D9, D11.

## Options
- **Synchrone, comme la référence** : simple, mais la page bloque tant que le SMTP répond (jusqu'à 10 s par destinataire) et rien n'est visible avant la fin.
- **Asynchrone avec file en base et travailleur** : robuste aux redémarrages, mais lourd pour quelques messages par mois.
- **Asynchrone en tâche de fond** (retenu) : le message et ses destinataires sont enregistrés dans une transaction, puis une tâche `tokio` fait les envois.

## Décision
- **Envoi (`dominet_server::messaging::send`)** : crée le message (`sending`) et un destinataire par compte avec `initial_statuses` (`pending` seulement si le canal est demandé **et** que le compte a l'adresse / le mobile, sinon `na` — écart voulu avec la référence qui marquait « échec : pas d'adresse »), puis rend la main. La page de détail s'ouvre aussitôt.
- **Tâche de fond** : un seul envoi à la fois pour tout le serveur (verrou global) ; pour chaque destinataire, l'e-mail part par `Channels::send_email` (sujet = sujet du message, texte brut, D11), puis le SMS est confié à l'appareil Android **en ligne le plus récemment vu**, de n'importe quel propriétaire (comme la référence). Sans appareil : `sms: aucun appareil Android connecte` ; appareil tombé entre-temps : `sms: appareil injoignable`. Un échec d'e-mail s'écrit `email: <raison>`.
- **SMS** : reste `pending` jusqu'à l'accusé de l'application, ou jusqu'à 60 s (ADR 0039, `expire_stale`). Le règlement passe par `android::settled`, qui met à jour le destinataire et clôt le message.
- **Fin** : le message passe à `sent` (avec `sent_at`) dès que plus rien n'est en attente (`finish_if_complete`, atomique). `sent` signifie « traité », pas « tout a réussi » : le tableau dit ce qui a échoué.
- **Audience** : `all` (comptes actifs), `staff` (administrateurs actifs), plus des comptes choisis ; cumul sans doublon (`resolve_audience`). Un compte inactif choisi explicitement est servi (comme la référence) mais le formulaire ne propose que les actifs.
- **En direct** : sujet `message:<id>` du WebSocket (`WsServerMsg::MessageProgress { message_id, complete }`), publié à chaque destinataire traité et à chaque accusé. Autorisé à l'expéditeur, aux destinataires et aux administrateurs ; message inconnu et message d'un autre reçoivent le même refus. Le navigateur relit la page à chaque évènement (comme les pages Android, ADR 0040). Les évènements ne sont pas conservés par le hub : la page relit de toute façon à l'ouverture.
- **Droits (D9)** : composer et lister les messages envoyés = administrateurs ; boîte de réception = tout compte ; détail = expéditeur, destinataire ou administrateur — l'expéditeur et les administrateurs voient le tableau complet, un destinataire seulement son propre état. Message inconnu ou étranger : « introuvable ».
- **Reprise au démarrage** : un message resté `sending` après un arrêt brutal voit ses envois en attente déclarés en échec (« interrompu par un redémarrage du serveur ») puis est clos (`recover_interrupted`), pour ne rien laisser « en cours » indéfiniment. On ne sait pas si l'envoi interrompu est parti : l'échec est la réponse prudente, l'expéditeur peut renvoyer.
- **Pages** : `/messaging` (envoyés), `/messaging/inbox`, `/messaging/compose`, `/messaging/<id>` ; les 100 derniers messages par liste (la référence paginait par 20).

## Vérification
- `crates/dominet/tests/messaging.rs` (6 tests, faux SMTP et téléphone qui parle comme l'application) : e-mails reçus avec expéditeur, sujet et texte ; SMS confié avec le bon numéro ; statuts avant et après l'accusé ; SMTP qui refuse et aucun téléphone ; SMS sans accusé échoué au bout de 60 s ; évènements jusqu'à `complete` ; droits d'abonnement (expéditeur, destinataire, administrateur, étranger, inconnu) ; reprise après redémarrage.
- Unitaire : composition de l'audience.
- **E2E `tests/e2e/messaging.py`** (31 vérifications) : formulaire refusé vide, envoi, suivi des statuts en direct avec accusé fait à la main, SMTP simulé (`tools/sim-smtp.py`), boîte de réception du destinataire sans le tableau des autres, pages réservées, échec sans téléphone avec sa raison.
- Aucun envoi réel : jamais d'hôte SMTP ni de Free Mobile joints par les tests (D1).

## Conséquences
- Un serveur qui s'arrête pendant un envoi peut avoir envoyé un e-mail que la base déclare en échec ; renvoyer est sans danger pour le serveur, mais le destinataire pourrait recevoir deux fois.
- L'envoi global est sérialisé : suffisant pour quelques destinataires, à revoir (file bornée, concurrence limitée) si les volumes changeaient.
- Pas de brouillons ni de pièces jointes (la référence n'en avait pas d'usage : l'état `draft` du schéma reste inutilisé).
