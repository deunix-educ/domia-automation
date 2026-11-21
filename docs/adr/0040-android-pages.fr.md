# 0040 — Pages Android : droits, jeton affiché une fois, mise à jour en direct


- Statut : accepté
- Date : 2026-11-07

## Contexte
Le relais (ADR 0039) reçoit SMS et appels ; il faut les pages qui les montrent (`/android/…`, adresses de la référence déjà dans la navigation), l'enregistrement des appareils et l'envoi de SMS depuis le navigateur. Plan R5 : D2, D3, D5, D9, D10, D12.

## Décision
- **Quatre pages** : tableau de bord `/android`, appareils `/android/devices`, SMS `/android/sms?device=N`, journal d'appels `/android/calls?device=N`. Les opérations (ajouter, supprimer, nouveau jeton, envoyer) sont des fonctions serveur, pas des pages de confirmation : la suppression demande une confirmation sur place.
- **Droits (D9)** : tout compte connecté ajoute ses appareils et voit les siens ; un administrateur voit tous les appareils (les dépôts prennent `Option<UserId>`, `None` = tous). Un appareil qui n'est pas au compte est « introuvable » (même réponse qu'un identifiant inconnu), pour les pages, l'envoi, la suppression et le nouveau jeton.
- **Jeton affiché une fois (D2)** : seules la création et la régénération le renvoient (`NewAndroidDeviceDto`, dont `Debug` masque le jeton) ; aucune liste, aucune page de rechargement ne le contient. Un bandeau le montre avec l'adresse à saisir et un bouton « masquer ».
- **En direct (D5)** : les pages suivent les sujets `android:<id>` de leurs appareils ; chaque évènement incrémente un compteur (`Realtime::android`) qui relance la lecture de la page. Choix simple : on relit au lieu de fusionner des évènements (100 lignes au plus, requêtes légères), donc l'écran reste identique à la base.
- **Envoi (D3)** : l'appareil doit être connecté (sinon refus, rien d'enregistré) ; le SMS sortant est enregistré, mis dans la file (`sms_outbox`) puis confié à l'appareil ; les autres onglets reçoivent `SmsQueued`. La page montre l'état de chaque SMS sortant (**en attente**, **envoyé**, **échec** avec sa raison) lu dans la file ; sans accusé au bout de 60 s : échec.
- **Lu à l'affichage** (comme la référence) : ouvrir la page des SMS marque lus les SMS reçus de l'appareil. Aucune purge, 100 lignes par page (D12).
- **Validation** : champ par champ (nom, numéro de l'appareil ; destinataire, message), mêmes règles que la référence (`SendSmsForm`).

## Vérification
- `tests/ui.rs` : droits par compte, validation, jeton rendu une fois (et absent des listes), marquage lu, suppression, régénération.
- `android_ws.rs` : un SMS écrit dans le navigateur atteint l'application et son état suit l'accusé (envoyé, échec avec raison).
- **E2E `tests/e2e/android.py`** avec un **téléphone simulé** (`tools/sim_android.py` : client WebSocket en bibliothèque standard, mêmes trames JSON que l'application Kotlin, réponses aux trames de contrôle comme OkHttp) : ajout depuis l'interface, présence, SMS et appels en direct, envoi avec accusé et échec, refus hors ligne, nouveau jeton (l'ancien est coupé en 4001), suppression.

## Conséquences
- L'adresse à saisir dans l'application est `ws://hôte/ws/android/device/<jeton>/` ; `wss://` viendra avec TLS (R7).
- Une page relit à chaque évènement : si le volume devenait grand, on fusionnerait les évènements côté navigateur.
