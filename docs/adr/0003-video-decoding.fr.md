# 0003 — Décodage vidéo : `ffmpeg` en sous-processus derrière `FrameSource` (spike 4)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO** pour `ffmpeg` (VAAPI si disponible, logiciel sinon) ; **NO-GO** pour `openh264` comme défaut

## Contexte
Mouvement, YOLO et snapshots ont besoin d'images décodées. Aucun décodeur H.264 100 % Rust mature
n'existe (`retina` ne décode pas). Candidats : `ffmpeg` en sous-processus (pipe de trames brutes),
`ffmpeg-next` (liaison C, nécessite les en-têtes `libav*-dev`, absents de l'hôte, non évalué),
`openh264` (Cisco, logiciel). Spike : `spike (code non conservé)` + banc `ffmpeg`, sur l'EliteDesk
**i5-6500 (Skylake, HD Graphics 530)**, `ffmpeg` 7.1.5, pilote VAAPI `iHD` 25.2.3, clips de 30 s
capturés sans réencodage sur les caméras réelles (Q3R 720p15, sous-flux 360p15, Xiaomi 1080p, Ascam 1080p12).

## Mesures (CPU en % d'un cœur, par flux, pour 1 s de vidéo)
Sur fichier, vitesse maximale (hors réseau) :

| Flux | Logiciel `-threads 1`, 5 fps → 640×640 RGB | VAAPI décodage seul | VAAPI + 5 fps + `hwdownload` + scale logiciel |
|---|---|---|---|
| Xiaomi 1080p | 7,6 % | 1,2 % | 3,5 % |
| Q3R 720p15 | 9,4 % | 0,9 % | 2,5 % |
| Ascam 1080p12 | 5,8 % | 1,1 % | 3,5 % |
| Q3R sous-flux 360p | 4,2 % | 0,7 % | 1,2 % |

En direct (RTSP/TCP depuis les caméras), **4 flux simultanés**, 5 fps vers 640×640 :
logiciel `-threads 1` = **87 % d'un cœur** (22 % de la machine) ; VAAPI + `hwdownload` + scale logiciel = **32 %** d'un cœur (8 % de la machine). Aucun échec.
Latence de la première image avec un `ffmpeg` lancé à froid : **2,1 à 4,3 s** (attente de l'image clé).
Décodage logiciel de toutes les images, pour référence : 1080p ≈ 5–9 % d'un cœur en rafale.

`openh264` (1 thread, image par image) : Q3R 720p = **19 %** d'un cœur (≈ 2× `ffmpeg` logiciel) ; **échecs sur les flux Xiaomi et Ascam** (183 erreurs sur 211 unités d'accès, « pas de jeu de paramètres » ; `ffmpeg` les décode sans erreur) même après regroupement en unités d'accès. Cause non élucidée dans le temps imparti ; coût d'intégration en plus : dépaquetage RTP (`retina`) et pas de décodage matériel.

Dans un conteneur Docker non-root (image `debian:trixie-slim` + `ffmpeg` + `intel-media-va-driver`), le décodage VAAPI fonctionne avec `--device /dev/dri` et le groupe `render` de l'hôte (GID 992 ici) ajouté à l'utilisateur.

## Décision
- Trait `FrameSource` (ROADMAP R6) ; implémentation par défaut : **un `ffmpeg` de longue durée par caméra**, lisant le **re-stream MediaMTX** (pas la caméra directe), qui écrit sur stdout des trames brutes de taille fixe à N fps (`-vf fps=N,scale=W:H,format=rgb24`) ; le processus est supervisé (redémarrage avec backoff) et ses trames sont lues par lots.
- **Accélération matérielle opportuniste** : au démarrage, sonde `ffmpeg -hwaccel vaapi … -f null` ; si elle réussit → `-hwaccel vaapi -hwaccel_output_format vaapi` puis `fps=N,hwdownload,format=nv12,scale=…` ; sinon repli logiciel `-threads 1`. Sur le Pi 5 (pas de décodage H.264 matériel), c'est le repli logiciel qui sert.
- Snapshots à la demande : ne pas lancer un `ffmpeg` à froid (2–4 s) ; servir la dernière trame du `FrameSource` déjà actif, ou, sans flux actif, décoder une image clé (`-skip_frame nokey`, ≈ 2 % d'un cœur).
- Docker : l'image embarque `ffmpeg` et `intel-media-va-driver` ; le Compose de prod ajoute `devices: [/dev/dri:/dev/dri]` et `group_add: ["${RENDER_GID}"]` (GID du groupe `render` de l'hôte, à lire par `make prod-init`).
- `openh264` et `ffmpeg-next` restent des implémentations de `FrameSource` possibles ultérieurement (R9), non retenues par défaut.

## Conséquences
- Le budget CPU des caméras est faible (≈ 8 % de la machine pour 4 caméras avec VAAPI, 22 % en logiciel) : **le goulot sera YOLO** (spike 5), pas le décodage.
- La résolution réelle du flux diffère parfois du profil ONVIF : la Q3R diffuse en **1280×720** alors que son profil annonce 1920×1080 ; sur l'Ascam, `/stream` est le **sous-flux 800×448** et `/stream1` le 1080p ; la Xiaomi n'émet qu'**≈ 7 images/s** dans le clip capturé. Les dimensions de trame doivent donc être lues sur le flux (`ffprobe`/sortie `ffmpeg`), pas déduites d'ONVIF.
- Le `fps=` en amont du `hwdownload` limite le coût de copie GPU→CPU ; c'est lui qui rend la chaîne VAAPI rentable.

## Pièges relevés
- **`scale_vaapi` échoue** sur cet hôte (« requested VAProfile is not supported » : pas de traitement vidéo VPP) ; la chaîne `hwdownload` + scale logiciel fonctionne. `libigfxcmrt7` (main, 25.2.3) a été installé le 2026-10-31 **sans effet** : l'hypothèse initiale est infirmée. Piste restante, non testée : `intel-media-va-driver-non-free` (composant `non-free` de Debian, non activé ici, remplace `intel-media-va-driver`) qui embarque les noyaux Intel non libres. Non retenu : la chaîne actuelle coûte 32 % d'un cœur pour 4 caméras ; l'image Docker n'utilise que le pilote libre.
- Un banc de test qui jette `stderr` peut mesurer une commande en échec : mes premiers chiffres « VAAPI + scale » étaient faux pour cette raison (corrigés ci-dessus).
- Un TS de copie (`-c copy`) d'un sous-flux Thingino exige `-bsf:v h264_mp4toannexb` ou un conteneur Matroska.
