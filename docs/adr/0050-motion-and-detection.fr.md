# 0050 — Mouvement et détection d'objets


- Statut : accepté
- Date : 2026-11-08

## Contexte
la référence (`detect_motion_camera`, `run_yolo_detection`, `OnnxBackend`) : pour chaque caméra **en mode `motion`**, toutes les 30 s, il rouvre le flux, lit ~20 images, compare la première et la dernière (gris, flou 21 × 21, `absdiff`, seuil 25, ouverture 5 × 5, rapport de pixels > `MOTION_THRESHOLD` 0,01), crée un `MotionEvent`, publie sur MQTT, notifie (délai par caméra 600 s) puis lance YOLOv8n si la caméra l'active (une image, classes et confiance de la caméra, image annotée, `DetectionEvent`, MQTT, notification 300 s). Plan R6 : D7 (un seul `FrameSource`, fenêtre glissante), D8 (modèle), D12 (mesures).

## Décision
- **Mouvement en Rust pur** (`dominet_surveillance::motion`, aucune dépendance à OpenCV) : même algorithme, avec les détails d'OpenCV reproduits (gris entier à 14 bits, flou séparable à bords en réflexion 101, σ dérivé de la taille du noyau, ouverture à bords neutres). **Parité mesurée** contre OpenCV 5 sur 8 paires d'images synthétiques (un script de portage retiré, `tests/motion_parity.rs`) : écart maximal **0,0002** du rapport (tolérance du test 0,0005) ; mêmes décisions au seuil de 0,01 pour chaque cas. Le flou 8 bits d'OpenCV est en virgule fixe, non reproduit bit à bit.
- **Fenêtre glissante (D7)** : le `FrameHub` garde les 20 dernières images de chaque caméra ; toutes les `MOTION_INTERVAL_SECS` (30), la première et la dernière sont comparées — **seules deux images sont décodées et floutées**. Aucun flux n'est rouvert (la référence : 2 à 4 s d'établissement à chaque tour). À 2 images/s la fenêtre couvre ~10 s (la référence : ~2 s à 10 images/s), donc un mouvement lent est plus facile à voir ; écart voulu, réglable par le seuil. Au moins 5 images et une dernière image de moins de 15 s, sinon pas de verdict.
- **Suites d'un mouvement** (comme la référence) : `MotionEvent`, MQTT `dominet/surveillance/<uuid>/motion` (`camera_uuid`, `camera_name`, `confidence` à 4 décimales, `timestamp`), e-mail + SMS `Mouvement détecté — <nom> (<lieu>) : x.x%` avec délai par caméra `MOTION_ALERT_COOLDOWN`, puis YOLO. Le rôle `video` publie par le client MQTT du rôle `mqtt` s'il tourne dans le processus, sinon par un client à lui (`<client_id>-video`).
- **YOLO** (`dominet_surveillance::yolo`, `ort` épinglé `=2.0.0-rc.15`, ADR 0009) : mêmes entrée / sortie / seuils que `OnnxBackend` (640 × 640, score ≥ 0,10 puis NMS 0,45 sans distinction de classe, seuil et classes de la caméra appliqués ensuite). Un pool de sessions à un fil de calcul (`YOLO_WORKERS`, défaut 1), chargé **à la première détection** (le modèle n'occupe pas la mémoire s'il est coupé). Entrée : la **dernière image du `FrameSource`** (JPEG de 1280 de large au plus), pas une image du flux brut comme la référence : la compression et la réduction ajoutent un léger écart, négligeable pour la classe, la boîte et le score (voir parité). **Parité mesurée** sur 4 images (échantillon public `bus.jpg` d'Ultralytics et trois variantes : redimensionnée, recadrée, sans objet) : **17/17** détections de `OnnxBackend` retrouvées (même classe, IoU > 0,9, écart de score < 0,02), aucune en trop.
- **Réglage global** : `DetectorConfig` (une ligne ; `enabled`, `save_frames`, `model_path`), modifié dans l'interface (`/surveillance/events`, administrateurs). Coupé par défaut. `save_frames` : image annotée `<uuid>_<AAAAMMJJ_HHMMSS>.jpg` dans `DETECTIONS_DIR`, servie par `GET /surveillance/detections/<id>/frame/` (tout compte connecté). **Écart** : les boîtes sont dessinées, **pas le texte** (aucune police embarquée).
- **Page des événements** `/surveillance/events` (tout compte connecté) : mouvements et détections récents (heure du serveur), image annotée quand elle existe. la référence n'avait pas de page.
- **Modèle (D8, tranché par l'utilisateur)** : `yolov8n.onnx` **n'est pas commité** ; `make fetch-model` le copie depuis le dépôt de référence (lecture seule) et vérifie son empreinte SHA-256. `make image` le récupère si absent et le `Dockerfile` l'embarque (`/models`). Sans modèle les tests de parité le disent et ne vérifient rien ; la détection journalise « modèle indisponible ».

## Vérification
- Unitaires : gris entier, noyau normalisé et σ d'OpenCV, réflexion 101, flou d'une image plate et d'un point, rapport (carrés, taches et lignes fines effacées par l'ouverture, seuil strict), NMS et mise à l'échelle des boîtes, IoU, prétraitement, annotation, filtre de classes et de confiance, délai par caméra.
- Parité : `motion_parity.rs` (OpenCV) et `yolo_parity.rs` (`OnnxBackend`).
- Intégration (vrais `ffmpeg`, vrai modèle, Mosquitto de dev, faux SMTP) : source fixe (aucun mouvement) et source qui oscille (mouvement, événement, MQTT, un seul e-mail malgré deux mouvements), absence de fenêtre ; détecteur coupé / YOLO coupé / classe non voulue (rien), classe voulue (événement, image annotée lisible, MQTT, e-mail), sans `save_frames` (pas de fichier).
- E2E `events` (vrai flux H.264 de l'image du bus, luminosité oscillante) : mouvement chiffré, détection « bus » avec image annotée servie en JPEG, page, coupure du détecteur depuis l'interface.

## Limites
- Les mesures de charge (CPU / RAM, nombre de caméras tenables) sont dans `docs/R6-mesures.md` : sur l'EliteDesk, **12 caméras 1080p/15 en mode complet (enregistrement, archive, mouvement, YOLO) = 45 % de la machine avec VAAPI** (≈ 1,5 cœur, 1,9 Go), 8 caméras = 60 % en logiciel.
- Le mode `motion` ne déclenche toujours pas l'enregistrement (voir ADR 0049) ; YOLO ne se déclenche que sur mouvement, comme la référence (pas d'échantillonnage périodique, bien que `DetectionTrigger::Periodic` existe dans le schéma).
- Le décodage VAAPI et le GPU ne servent pas à YOLO (ADR 0009).

## Complément (lot 9) : image Docker
La bibliothèque ONNX Runtime précompilée par `ort` est en C++ : la construction exige `g++`, l'exécution `libstdc++6`, et l'ensemble demande une glibc ≥ 2.38. L'image passe donc de Debian 12 (*bookworm*) à **Debian 13** (*trixie*), identique à la cible. Découvert par `make ci-full`, qui n'avait pas été relancé après le changement du `Dockerfile`.
