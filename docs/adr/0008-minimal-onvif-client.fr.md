# 0008 — ONVIF : client SOAP minimal maison (spike 6)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO** (avec réserves ci-dessous)

## Contexte
Pas de crate ONVIF mature adaptée (ROADMAP §10). L'implémentation de référence s'appuie sur `onvif-zeep`.
Il faut : WS-Discovery, identité/capacités/profils, URI RTSP, PTZ (`ContinuousMove`/`Stop`/`GetStatus`,
presets), imaging (`IrCutFilter`). Spike : `spike (code non conservé)` (`reqwest` + `roxmltree` + `sha1`/`base64`
+ `socket2`), exécuté depuis l'EliteDesk contre 4 vraies caméras (acces dans `deploy/.env.cameras`).

## Résultats
| Caméra | Firmware | ONVIF | Résultat |
|---|---|---|---|
| 192.168.1.60 Jooan Q3R | Thingino stable | Device/Media/PTZ/Imaging | tout OK ; PTZ pan réel (0 → 46,4°) ; IR-cut `Auto → ON → Auto` |
| 192.168.1.42 Jooan A2R | Thingino stable | idem | tout OK ; PTZ 0 → 46,0° ; IR-cut OK |
| 192.168.1.30 Xiaomi | Thingino master | Device/Media seulement | profils + RTSP OK ; pas de PTZ ni d'imaging (normal) |
| 192.168.1.50 Ascam (Vatilon PB1) | firmware constructeur | Device/Media/PTZ/Imaging | tout OK ; IR-cut `ON → OFF → ON` ; adresses de services sur `:80/onvif/Media`… |
| 192.168.1.29 « NVT IPC » (`IPC_NT98566`, ONVIF 16.12) | firmware NVT | Device/Media/PTZ/Imaging sur **`:8899`** (`/onvif/Media`, `/onvif/PTZ`, `/onvif/imaging`) | auth OK (`admin`) ; profils 4096×2160 et 704×576 ; IR-cut `AUTO` ; **PTZ réel confirmé** par comparaison d'images (la caméra tourne), mais **`GetStatus` renvoie toujours 0,0** : aucune position lisible ; butées mécaniques (un `ContinuousMove` long l'amène en bout de course) ; l'interface web d'administration de la caméra est inutilisable (plugin trop ancien) : tout passe par ONVIF/RTSP |
| 192.168.1.38 OpenIPC | OpenIPC | **aucun ONVIF** | le 401 vient de l'interface web : contrôle par l'API OpenIPC (backend night `openipc`) |

- Authentification **WS-Security UsernameToken, PasswordDigest** (`base64(sha1(nonce + created + mot de passe))`) acceptée par les trois familles ; l'horloge de la caméra est lue d'abord par `GetSystemDateAndTime` (sans auth) pour corriger `Created` (décalage mesuré : 0 s ici).
- Latences unicast : 12–110 ms par appel (Thingino ≈ 15–50 ms, Ascam ≈ 50–90 ms).
- **WS-Discovery** : depuis l'hôte, 4 appareils trouvés sur 4 essais avec 3 probes espacés d'1 s et 6 s d'écoute (1 seul probe/4 s : 3 sur 4 une fois). La Q3R (`.60`) **ne répond jamais** au multicast malgré un ONVIF unicast fonctionnel → l'ajout manuel par IP (`add-manual`) reste indispensable. Un appareil non répertorié (`192.168.1.29`, « NVT IPC-model », port 8899) a aussi répondu.
- PTZ : la vitesse n'est pas proportionnelle à `x` sur Thingino (≈ 100°/s dès qu'on est non nul) ; la précision d'un déplacement dépend du délai entre `ContinuousMove` et `Stop` (≈ 20–100 ms de gigue HTTP) → ± 3° après un aller-retour. `GetStatus` renvoie la position en degrés sur Thingino.

- **`GetStreamUri` de la caméra `.29` renvoie une URL qui contient le nom d'utilisateur et le mot de passe en clair** (`rtsp://…/user=admin_password=…_channel=1_stream=0…`). Ne jamais journaliser ni afficher cette valeur ; la masquer systématiquement (`password=***`).
- Le port du service ONVIF n'est pas toujours 80 (ici 8899, repéré par WS-Discovery via `XAddrs`) : l'URL du service est un paramètre de la caméra, la découverte la renseigne.

## Décision
- `dominet-surveillance::onvif` : client SOAP maison (enveloppe à la main, réponses analysées par `roxmltree`, on retient `roxmltree` plutôt que `quick-xml` pour son DOM en lecture seule), limité aux appels utilisés : `GetSystemDateAndTime`, `GetDeviceInformation`, `GetCapabilities`, `GetProfiles`, `GetStreamUri`, PTZ (`ContinuousMove`, `Stop`, `GetStatus`, presets), imaging (`Get/SetImagingSettings`).
- **Toujours** s'authentifier en WS-Security digest avec correction d'horloge ; ne jamais afficher ni journaliser les mots de passe (le spike n'affiche que l'utilisateur).
- **Un client HTTP sans réutilisation de connexion** (`pool_max_idle_per_host(0)`) : le serveur ONVIF de Thingino ferme la connexion après chaque réponse ; la réutilisation provoque `connection closed before message completed` sur l'appel suivant.
- Les adresses de services viennent de `GetCapabilities`, jamais déduites de l'hôte (l'Ascam utilise `/onvif/Media`, Thingino `/onvif/media_service`).
- Découverte : 3 probes à 1 s d'intervalle, 6 s d'écoute, interface choisie par la route par défaut ; rôle `dominet-discovery` en réseau hôte (conforme §2.2).
- Tests de contrat : réponses SOAP de ces caméras à capturer (sans secrets) dans `tests/fixtures/onvif/` au début de R6.

## Conséquences
- Un contrôle PTZ fluide passe par `ContinuousMove` suivi de `Stop` à la relâche de la touche (comme l'UI de référence), pas par des déplacements calibrés au temps.
- Les caméras sans ONVIF (OpenIPC) exigent un backend dédié (`NightControl::openipc`) et une saisie manuelle des paramètres.
- Non couvert : `GetPresets`/`SetPreset`/`GotoPreset`, événements ONVIF, `SetSystemDateAndTime`, authentification HTTP Digest (utile pour le backend `goform`, à valider en R6 sur une caméra concernée).
