# ADR 0064 — Visionnage des caméras authentifié : MediaMTX interroge DomiNet, jetons de visionnage par compte et identifiants de flux personnels

Statut : **accepté** (2026-11-24, validé par le propriétaire). Rien de ce qui suit n'est encore implémenté ; le plan liste les lots, chacun validé avant le suivant. Clôt le point « identifiants de caméra par spectateur » de la feuille de route et de [À améliorer](../to-improve.md).

## Contexte
Aujourd'hui, toute personne connectée au réseau domestique peut regarder toutes les caméras : MediaMTX autorise `read` et `playback` à tout le monde (`authInternalUsers`, `user: any`, sans mot de passe), en RTSP (8554) et en WebRTC (8189, et WHEP par nginx sur `/webrtc/`). Seuls la publication, l'API et les métriques sont réservées au réseau Docker et à la machine. L'interface, elle, est protégée par des comptes (tout compte connecté peut regarder) : la vidéo est donc le seul endroit où le système de comptes ne s'applique pas. [Sécurité](../security.md) documente cet écart et conseille de mettre les invités sur un autre réseau.

Ce que le propriétaire a décidé (2026-11-23) : l'authentification est **déléguée à DomiNet**, **tout compte connecté peut regarder toutes les caméras** (pas de droits par caméra), et chaque compte peut obtenir des **identifiants personnels et révocables** pour les lecteurs externes (VLC, applications du téléphone).

Ce que dit la documentation de MediaMTX (lue le 2026-11-23 ; la page n'est pas versionnée et est probablement plus récente que la 1.21.3, donc tout point marqué *à vérifier* doit l'être d'abord sur le vrai binaire 1.21.3) :
- `authMethod: http` avec `authHTTPAddress` : MediaMTX envoie en POST `{user, password, token, ip, action, path, protocol, id, query}` ; un statut `20x` autorise, tout autre refuse. Un corps optionnel `{"expireAt": "<RFC 3339>"}` ferme la session à cette date.
- Les clients RTSP envoient d'abord des identifiants vides ; le serveur doit répondre `401` pour qu'ils les envoient. Les navigateurs et clients WHEP peuvent envoyer `Authorization: Basic …` ou `Authorization: Bearer <jeton>`.
- `authHTTPExclude` évite l'appel HTTP pour certaines actions (`api`, `metrics`, `pprof` par défaut). *À vérifier :* si `publish` peut être exclu, si `authInternalUsers` s'applique encore avec `authMethod: http`, et si un jeton Bearer seul arrive dans `token` ou dans `password`.
- Aucune mise en cache des décisions n'est documentée : un appel HTTP par authentification, au moins deux pour une session RTSP.

## Options
- **A. MediaMTX interroge DomiNet (`authMethod: http`)** (choisie). Un seul endroit décide, les comptes et la révocation sont ceux de l'application, le navigateur et les lecteurs externes sont couverts.
- **B. `auth_request` de nginx sur `/webrtc/` seulement.** Simple, mais RTSP (8554) et ICE (8189) restent ouverts sur le LAN : la moitié de l'écart demeure.
- **C. Un mot de passe de lecture partagé dans la configuration de MediaMTX.** Trivial, mais pas « par spectateur » : impossible de couper une personne sans le changer pour tous.
- **D. Un utilisateur interne MediaMTX par spectateur, écrit dans le fichier de configuration.** Demande une réécriture et un rechargement à chaque changement, et duplique la base des comptes.

## Décision
Option A.
- **MediaMTX** tourne avec `authMethod: http` et `authHTTPAddress: http://dominet:8000/internal/mediamtx/auth`. La route n'existe que côté réseau Docker : nginx ne la relaie pas, et le gestionnaire refuse tout appelant hors du réseau Docker.
- **La décision** (gestionnaire `POST`) n'autorise un `read` / `playback` sur un path de caméra que pour un identifiant valide d'un compte actif ; `publish`, `api`, `metrics` et `pprof` ne viennent que de l'intérieur du réseau Docker ; des identifiants vides reçoivent `401` (le défi RTSP) ; tout le reste est refusé. Un refus est compté dans une métrique et journalisé sans aucun secret. Les échecs Basic comptent dans le même blocage que la connexion (ADR 0004).
- **Navigateur :** à l'ouverture de la page live, une fonction serveur (session exigée) crée un **jeton de visionnage** : 256 bits aléatoires, stocké haché (SHA-256), lié au compte, valable quelques minutes. Le client WHEP l'envoie en `Authorization: Bearer`. Il est renouvelé avant son expiration.
- **Lecteurs externes :** une page de compte (« Mon compte » → « Accès aux caméras ») crée un **identifiant de flux personnel** : le nom d'utilisateur du compte et un mot de passe généré montré une seule fois, stocké haché, révocable, avec une date de dernier usage. La page affiche l'adresse RTSP à coller dans VLC. Un compte peut en avoir plusieurs (un par appareil).
- **Même modèle de stockage que les jetons MCP** ([ADR 0059](0059-mcp-access-tokens.md)) : table avec empreinte, propriétaire, nature, dates de création et d'expiration, date de révocation. Aucune clé de signature à gérer (le réglage `SECRET_KEY` a été supprimé).
- **Les lectures de DomiNet lui-même** (le rôle `video` décode le re-stream de MediaMTX) utilisent un **identifiant de service** généré par `make prod-init` (`MEDIAMTX_READ_USER`, `MEDIAMTX_READ_PASSWORD`), pas une exception par adresse IP. Faire confiance à la plage du réseau Docker pour `read` serait faux : selon le mode du proxy Docker, le trafic du LAN peut sembler venir de l'adresse du pont.
- **Révocation :** effective à la connexion suivante. Pour une session ouverte, DomiNet la coupe par l'API de MediaMTX (`/v3/*conns/kick`, `/v3/webrtcsessions/kick`), à l'aide de l'`id` de session que MediaMTX envoie dans l'appel d'authentification. Si cela s'avère peu fiable, `expireAt` borne la session à la place.

## Conséquences
- **Changement incompatible pour les spectateurs :** VLC et les applications du téléphone devront recevoir un identifiant personnel avant de pouvoir regarder. La note de version et [Sécurité](../security.md) le disent, avec les étapes de migration.
- **DomiNet devient une dépendance du visionnage.** S'il est arrêté, MediaMTX refuse les lectures (échec fermé). L'enregistrement n'est pas touché : les caméras sont tirées par MediaMTX et écrites par l'enregistreur à l'intérieur du réseau Docker.
- Un appel HTTP de plus par session (deux en RTSP) ; négligeable à cette échelle.
- Le plan média (ICE, 8189) n'est pas authentifié en soi : une session WebRTC est autorisée au moment où l'offre WHEP est postée. Rediriger 8189 ou 8554 vers Internet reste déconseillé.
- Hors périmètre : droits par caméra, authentification Digest, HLS (désactivé), chiffrement de bout en bout du RTSP.

## Plan (un lot à la fois, chacun validé avant le suivant)
0. **Spike sur MediaMTX 1.21.3** (pile de dev) : trancher chaque point marqué *à vérifier*, avec VLC et un navigateur. Si un point échoue, cet ADR est amendé avant tout code.
1. Table, migration et règles du domaine (jetons, identifiants, expiration), avec tests.
2. Route d'authentification, configuration de MediaMTX (production, démo, dev), identifiant de service, scripts d'installation et de migration.
3. Parcours navigateur : jeton de visionnage et client WHEP, test de bout en bout sur la page live.
4. Page « Accès aux caméras », révocation et coupure des sessions.
5. Documentation (EN et FR), note de version, retrait des entrées de la feuille de route et de « À améliorer ».
