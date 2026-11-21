# 0010 — Live caméra : client WHEP en Rust/WASM vers MediaMTX (spike 3)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO** (réserve levée le 2026-10-31 : live 720p validé par l'utilisateur dans son navigateur, voir ci-dessous)

## Contexte
Le live passe par WebRTC/WHEP : MediaMTX ré-expose les flux RTSP des caméras, et le front (WASM)
négocie la session. Spike : `spike (code non conservé)` (≈ 140 lignes, `web-sys`
`RTCPeerConnection` + `fetch` + `getStats`), intégré à la page Leptos du spike 1 ; MediaMTX 1.21.3 en
dev ; caméra réelle Jooan Q3R (Thingino, H.264 High, ingestion RTSP/TCP en deux chemins : 720p et 360p).

## Résultats
- **Signalisation côté serveur** : MediaMTX ingère les deux chemins de caméra (`ready`, piste H264) et répond **201 + SDP H.264** (`packetization-mode=1`) à une offre WHEP H.264 (testé par `POST` direct) pour les deux chemins.
- **Client WASM** : négociation complète (offre, collecte ICE, `POST`, réponse, `ontrack` → `<video>`) en **51 ms**, 25 images/s, 0 image perdue, tampon de gigue moyen **11 ms**, gigue 0,6 ms, RTT 0 ms (boucle locale). Mesuré sur une source de test VP8 publiée dans MediaMTX.
- **Latence de bout en bout** (source de test VP8 encodant l'heure murale dans l'image ; mesure dans le navigateur par `canvas`, 120 échantillons) : **médiane 40 ms, p95 59 ms**, min 19 ms, max 59 ms — génération + encodage VP8 + RTSP + MediaMTX + WebRTC + décodage + rendu, sur la boucle locale.
- **Première image** : 1,5 s après le clic, dominée par l'attente d'une image clé (GOP de 1 s de la source de test).
- **Limite du test** : le Firefox ESR de Debian (153) n'annonce **ni H.264 dans son offre WebRTC** (VP8, VP9, AV1 seulement) ; les préférences `media.peerconnection.video.h264_enabled` et GMP OpenH264 n'y changent rien en mode headless. Le décodage H.264 d'une caméra réelle dans un navigateur n'a donc **pas pu être vérifié automatiquement** ; la latence caméra réelle (encodeur de la caméra inclus) non plus.

- **Reproduit dans le navigateur de l'utilisateur** (Firefox ESR de Debian dans la session VNC de l'EliteDesk) : « WHEP HTTP 400 », journal MediaMTX `closed: codecs not supported by client`. Cause : le paquet Debian `firefox-esr` désactive OpenH264 par défaut (`/etc/firefox-esr/firefox-esr.js` : `pref("media.gmp-gmpopenh264.enabled", false)`), donc l'offre WebRTC ne propose pas H.264. Ce n'est pas un défaut du client WHEP.
- Un profil de test avec ce réglage forcé n'a pas téléchargé le plugin Cisco dans le temps imparti (aucun répertoire `gmp-gmpopenh264`).

## Décision
- `dominet-web::whep` reprend le module du spike (offre `recvonly`, collecte ICE ≤ 2 s, `POST application/sdp`, `DELETE` de la ressource à la fermeture à ajouter) ; l'URL WHEP est construite à partir des réglages `MEDIAMTX_WEBRTC_SCHEME` / `MEDIAMTX_WEBRTC_DOMAIN` (comme dans la référence), derrière nginx `/webrtc/` en prod (même origine, pas de CORS).
- Statistiques `getStats` (images décodées/perdues, fps, tampon de gigue, RTT) affichées dans le widget caméra et utilisées pour la reconnexion automatique (aucune image décodée pendant N s → nouvelle session).
- Pas de transcodage dans MediaMTX : le navigateur doit décoder le H.264 de la caméra. Pour les navigateurs sans H.264 WebRTC, repli prévu : flux MJPEG/snapshots rafraîchis via `FrameSource` (ADR 0003).
- **À valider sur un navigateur qui décode le H.264 en WebRTC** (Chrome, Edge, Safari, Firefox de Mozilla ; ou Firefox ESR Debian avec `media.gmp-gmpopenh264.enabled = true` et le plugin installé). Un Firefox ESR Debian sans réglage affichera toujours l'erreur 400 : prévoir un message explicite dans l'UI (« ce navigateur ne décode pas le H.264 ») plutôt qu'un échec muet.

## Conséquences / pièges
- **Port ICE publié à l'identique** : MediaMTX annonce au navigateur son port interne (8189) ; le Compose doit publier `8189:8189` en UDP **et** TCP sans décalage (un `18189:8189` casse la connexion), et `webrtcAdditionalHosts` doit lister l'adresse que le navigateur atteint (127.0.0.1 en dev ; domaine/IP du LAN et du VPN en prod). La stack de dev est corrigée en conséquence.
- Le délai d'affichage dépend du **GOP de la caméra** : MediaMTX ne peut pas demander une image clé à une source RTSP. À mesurer sur les caméras réelles (Thingino/Ascam) au début de R6 ; préférer des GOP ≤ 2 s.
- Les identifiants des caméras n'apparaissent dans aucun fichier du dépôt : les chemins MediaMTX du spike ont été créés à chaud par l'API (`/v3/config/paths/add`), la config de prod les générera depuis la base (`mediamtx_sync`).
- L'API `web-sys` est verbeuse mais stable ; chaque type `Rtc*` demande sa feature Cargo (liste dans `spike (code non conservé)`).
