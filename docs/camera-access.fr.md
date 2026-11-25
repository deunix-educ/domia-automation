# Accès aux caméras

Regarder une caméra demande un compte. MediaMTX, qui sert la vidéo, interroge DomiNet avant **chaque** lecture (RTSP et WebRTC), et DomiNet ne dit oui que sur la preuve d'un compte connecté. Une personne de votre réseau qui n'a pas de compte ne voit rien, et l'accès de l'une peut être coupé sans toucher aux autres. La conception est dans l'[ADR 0064](adr/0064-authenticated-camera-viewing.md).

Chaque compte voit toutes les caméras : il n'y a pas de droits par caméra.

## Dans le navigateur

Rien à faire. À l'ouverture d'une vue en direct, la page demande à DomiNet un **jeton de visionnage** lié à votre session (valable cinq minutes, que seul le navigateur détient) et l'envoie à MediaMTX avec la demande. Si vous êtes déconnecté, la vue ne démarre pas. Le jeton ne sert qu'à démarrer une vue : une vue en cours n'est pas coupée quand le jeton expire.

## Dans VLC, une application du téléphone ou tout lecteur RTSP

1. Connectez-vous, puis ouvrez **Mon compte → Gérer mon accès aux caméras** (`/account/cameras`).
2. Donnez un nom à l'appareil (par exemple « Téléphone d'Anna ») et une validité (30, 90, 180 ou 365 jours), puis **Créer le mot de passe**.
3. **Copiez le mot de passe tout de suite** : il n'est montré qu'une fois et ne peut pas être relu ; seule son empreinte est conservée.
4. Dans le lecteur, ouvrez `rtsp://<hôte>:8554/<uuid de la caméra>` (la page liste l'adresse de chaque caméra active) et donnez votre **nom d'utilisateur** (celui du compte) et le **mot de passe** de l'appareil.

**Quel lecteur.** `ffplay`, `ffmpeg` et `mpv` fonctionnent (`mpv "rtsp://<utilisateur>:<mot de passe>@<hôte>:8554/<uuid de la caméra>"`). Le **paquet VLC de Debian 13 ne sait pas lire le RTSP du tout**, avec ou sans mot de passe : Debian le compile sans le module `live555`, et VLC se rabat sur un ancien dialecte RTSP que MediaMTX refuse (`invalid SETUP path` dans son journal). Cela a été reproduit sur un MediaMTX sans authentification : cela n'a rien à voir avec cette fonction. Utilisez un autre lecteur, ou VLC en Flatpak (non essayé ici). Les applications du téléphone n'ont pas été essayées non plus.

Créez un mot de passe par appareil. Si un appareil est perdu, **révoquez** son mot de passe : il est refusé dès la prochaine connexion, et les sessions que votre compte a ouvertes sont fermées aussitôt. Un autre de vos appareils, avec son propre mot de passe, se reconnecte tout seul.

La colonne *Dernière utilisation* indique si un mot de passe sert encore avant de le révoquer.

## Ce que l'administrateur doit savoir

- **Mise à jour d'une installation** venant d'une version sans cette fonction : `make prod-update` (ou `make prod-up`) ajoute `MEDIAMTX_READ_USER` et `MEDIAMTX_READ_PASSWORD` à `.env`. C'est le compte propre à DomiNet pour décoder la vidéo (images, mouvement, détection) ; une installation neuve l'obtient par `make prod-init`. `make prod-doctor` bloque s'ils manquent. Rien à taper.
- **Changement incompatible pour les spectateurs** : un lecteur qui ouvrait `rtsp://<hôte>:8554/<uuid>` sans mot de passe est refusé jusqu'à ce que la personne crée un mot de passe sur cette page.
- **DomiNet est désormais nécessaire pour regarder** : s'il est arrêté, MediaMTX refuse toute lecture. L'enregistrement n'est pas touché (MediaMTX tire les caméras et l'enregistreur tourne dans le réseau Docker).
- **Blocage** : les échecs avec un nom d'utilisateur comptent dans le même blocage que la connexion (cinq échecs pour un nom et une adresse : refus pendant 15 minutes, même avec le bon mot de passe). Un lecteur qui a un mauvais mot de passe et réessaie en boucle se bloque lui-même.
- **Désactiver ou supprimer un compte** le refuse dès sa prochaine connexion ; une session déjà ouverte n'est pas coupée. Révoquez d'abord ses mots de passe si cela compte.
- **Les sessions du navigateur** ouvertes avec un jeton de visionnage n'ont pas de nom d'utilisateur : révoquer un mot de passe ne les ferme pas ; elles s'arrêtent avec la page.
- **Les ports 8554 et 8189 ne sont toujours pas faits pour Internet** ([Sécurité](security.md)). Un lecteur qui se connecte depuis l'extérieur reste un risque, avec un mot de passe devant.
- **Démo** : la pile de démonstration publie le RTSP sur 8555 et non 8554 ; l'adresse affichée par la page demande alors ce port.
- **Pile de développement** : le MediaMTX de dev reste ouvert (`make dev-up`) ; le comportement authentifié est testé par `tools/e2e.sh live_auth`, qui démarre un MediaMTX jetable avec la configuration livrée.

## Si un lecteur est refusé

| Symptôme | Cause probable |
|---|---|
| `401 Unauthorized` tout de suite | pas de mot de passe, mauvais nom d'utilisateur, ou mot de passe révoqué ou expiré (la page montre l'état) |
| Ça marchait, refusé pendant un moment | trop d'échecs : attendez 15 minutes, puis vérifiez le mot de passe |
| Toutes les lectures refusées, navigateur compris | DomiNet est arrêté ou injoignable depuis MediaMTX : `make prod-doctor` |
| Le navigateur marche mais le rôle vidéo n'a aucune image | `MEDIAMTX_READ_USER` ou `MEDIAMTX_READ_PASSWORD` est vide : `make prod-up` |
