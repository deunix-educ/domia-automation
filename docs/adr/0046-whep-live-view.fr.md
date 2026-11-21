# 0046 — Live d'une caméra : lecteur WHEP en Rust/WASM


- Statut : accepté
- Date : 2026-11-08

## Contexte
ADR 0010 (spike 3) : le navigateur négocie une session WebRTC avec MediaMTX par WHEP ; ce lot en fait la fonction de l'application : page live, widget du tableau de bord, reconnexion, sujet WebSocket `camera:<uuid>`. Plan R6 : D6 (navigateur sans H.264), D9 (droits).

## Décision
- **Lecteur** (`dominet_web::live::WhepPlayer`, client dans `whep.rs`, `web-sys`) : offre `recvonly`, collecte ICE (2 s au plus), `POST application/sdp`, piste attachée au `<video>` ; **à la fermeture, `DELETE` de la ressource de session** (en-tête `Location`), pour que MediaMTX libère la connexion tout de suite. La partie décisionnelle est pure et testée sur la machine (`StallDetector`, `failure_for_status`).
- **Reconnexion par `getStats`** : une lecture toutes les 2 s ; **aucune nouvelle image décodée pendant 8 s** (15 s pour la première, le temps d'une image clé) → session fermée et relancée, avec le délai croissant du WebSocket (1 s doublant jusqu'à 30 s, plus 25 % d'aléa). Le lecteur dit ce qu'il fait : « connexion », « flux perdu : reconnexion », « pas de flux ».
- **Navigateur sans H.264 (D6)** : `RTCRtpReceiver.getCapabilities` est interrogé **avant** toute tentative ; sans H.264, un message explicite (et le nom des navigateurs qui conviennent) remplace l'échec muet. Si MediaMTX répond `400` à un navigateur sans H.264, même message. *Repli par snapshots rafraîchis* : repoussé au lot 6 (il a besoin des snapshots).
- **Adresse WHEP** : `<MEDIAMTX_WEBRTC_SCHEME>://<MEDIAMTX_WEBRTC_DOMAIN>/<uuid>/whep`, avec le **mécanisme de la référence** : si le domaine configuré est un hôte propre de l'instance (`ALLOWED_HOSTS` sans `localhost`, `127.0.0.1`, jokers), il est remplacé par l'hôte par lequel le navigateur est arrivé, schéma, port et chemin conservés (`dominet_domain::public_url`, testé). Un schéma autre que `http` / `https` est refusé.
- **CSP** : `connect-src` ajoute **l'origine de MediaMTX** quand elle diffère de celle de la page ; en production, derrière nginx `/webrtc/` (même origine), rien à ajouter. La page n'ouvre aucune autre origine.
- **Droits (D9)** : tout compte connecté regarde une caméra **active** (`camera_live`, page `/surveillance/live/<uuid>`, widget `camera`). Une caméra inactive ou inconnue donne la même réponse. Le navigateur ne reçoit **jamais** l'adresse du flux source ni un identifiant : seulement l'adresse WHEP de MediaMTX.
- **Sujet WebSocket `camera:<uuid>`** : accepté pour tout compte connecté si la caméra existe et est active (même refus pour une caméra inconnue et une inactive) ; les évènements (mouvement, détection) arrivent aux lots 7 et 8. Le sujet reste écrit avec l'UUID à tirets, forme canonique du contrat, alors que l'identifiant public de la caméra s'écrit en 32 hexadécimaux.
- **Widget** : `WidgetDto.camera` ; un `<video>` par affichage (`instance` distingue deux widgets de la même caméra) ; une caméra introuvable ou inactive affiche « caméra introuvable ou inactive ».

## Vérification
- **Test de bout en bout avec un vrai flux H.264** (`tests/e2e/live.py`, dans `make e2e`) : `ffmpeg` publie une mire H.264 dans le MediaMTX de dev, la caméra la tire, et **Chromium sans écran** (protocole DevTools, `websockets`) ouvre la page : état « live », résolution 640×360, temps de lecture qui avance, statistiques affichées ; **on coupe le flux** (le lecteur le signale) **puis on le rétablit** (il se reconnecte seul) ; quitter la page ferme la session chez MediaMTX. Firefox ESR de Debian (sans H.264) y montre le message explicite. Sans Chromium, `ffmpeg` ou `websockets`, le script le dit et ne vérifie que la partie Firefox.
- Tests Rust : adresse WHEP (hôtes propres), CSP, page et fonction serveur (droits, caméra inactive, aucun secret), sujet WebSocket, widget.

## Limites
- Chromium sans écran valide le **chemin réseau et le décodage** ; l'**aspect** et la fluidité se jugent dans votre navigateur (Brave, Chromium).
- La latence de bout en bout avec une vraie caméra n'est pas mesurée ici (la mire H.264 est encodée localement) ; elle dépend du GOP de la caméra (ADR 0010).
- Le WASM de développement (48 Mo) retarde l'hydratation : le lecteur démarre quand la page est hydratée (`RELEASE=1 make dev-run`).
