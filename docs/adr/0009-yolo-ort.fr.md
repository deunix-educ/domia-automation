# 0009 — Détection d'objets : YOLOv8n via `ort` (spike 5)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO avec réserves** (`ort` n'a pas de version stable ; GPU Intel non évalué)

## Contexte
Reproduire `OnnxBackend` : YOLOv8n ONNX (12,9 Mo), entrée
640×640 NCHW float32 RGB/255, sortie `[1,84,8400]`, score ≥ 0,10, NMS IoU 0,45 (sans distinction de
classe). Spike : `spike (code non conservé)` (`ort` 2.0.0-rc.15, ONNX Runtime 1.31.0 précompilé) ; référence
la référence : `onnxruntime` 1.31.1 CPU. Machine : EliteDesk **i5-6500** (Skylake, 4 cœurs, AVX2).

## Mesures (médiane de 60 passes, image 640×640 d'entrée)
| Threads intra-op | Inférence `ort` (Rust) | Inférence de référence | Total Rust (pré + inf + post) |
|---|---|---|---|
| 1 | 105,6 ms | 109,0 ms | 112 ms |
| 2 | 60,0 ms | 62,4 ms | 67 ms |
| 4 | 42,8 ms | 41,7 ms | 50 ms |

- Pré-traitement Rust (resize bilinéaire direct + normalisation + NCHW) ≈ 4,5 ms ; post-traitement + NMS ≈ 2 ms.
- **Parité : 10/10 détections** de la référence retrouvées (même classe, IoU des boîtes > 0,9, écart de score < 0,02) sur 5 images (image COCO « bus » : 4 personnes + bus + parapluie ; 3 images de caméras réelles).
- OpenVINO (essai via `onnxruntime-openvino` la référence) : fournisseur **CPU** 38,6 ms contre 41,7 ms (+ ≈ 8 %, négligeable) ; fournisseur **GPU** inutilisable ici (runtime OpenCL Intel `intel-opencl-icd` absent) → repli CPU. Non évalué sur l'iGPU.
- Binaire de test : 31 Mo (ONNX Runtime lié statiquement), dépendances dynamiques `libstdc++`, `libgcc_s`, `libm`, `libc`. Création de session : ≈ 75 ms.
- Binaires ONNX Runtime précompilés pour `ort` : **x86_64-linux-gnu** et **aarch64-linux-gnu** (CPU) ; **pas de musl** → image glibc (Debian slim). Variante `openvino` précompilée disponible pour x86_64-linux-gnu.

## Décision
- `dominet-surveillance::yolo` : `ort` (CPU), pré/post-traitement en Rust purs (code du spike repris), même contrat que la référence (`score ≥ 0,10` avant NMS, le seuil utilisateur `yolo_min_confidence` appliqué par l'appelant).
- **Version épinglée exactement** (`=2.0.0-rc.15`) : `ort` n'a que des versions candidates (API qui bouge : `Session::builder` exige désormais un `Environment`). Montée de version = tâche explicite avec relance du test de parité. Features : `std`, `ndarray`, `download-binaries`, `tls-rustls`, `copy-dylibs` (le défaut `tls-native` exigerait OpenSSL, absent).
- **Un pool de workers à 1 thread intra-op chacun**, plutôt qu'une session multi-threads : c'est le plus efficace par cœur (0,112 cœur·s par inférence contre 0,20 à 4 threads). Cible de débit : **≈ 9 inférences/s par cœur** dédié, soit ≈ 18/s avec 2 cœurs.
- **Budget** : YOLO échantillonné à 1–2 images/s par caméra (pas à la cadence vidéo) ; 4 à 5 caméras à 2 images/s ≈ 1 cœur de YOLO + ≈ 8 % de la machine pour le décodage VAAPI (ADR 0003). Le nombre de workers et la fréquence deviennent des réglages (`AppSetting`).
- Image Docker glibc, `models/yolov8n.onnx` copié dans l'image, chemin via `YOLO_MODEL_PATH`.
- Test de non-régression : l'image COCO de référence et ses détections (sans donnée sensible) entrent dans `tests/fixtures/yolo/` en R6 ; le test de parité compare à ces valeurs figées.

## Conséquences
- Latence mono-image acceptable pour de la détection d'événements, pas pour du suivi à la cadence vidéo : cohérent avec l'usage (alertes, `DetectionEvent`).
- Le gain potentiel de l'iGPU (HD 530, 24 EU) est probablement modeste face à un CPU AVX2 et entre en concurrence avec VAAPI. `intel-opencl-icd` **n'existe pas dans Debian 13** (ni `main`, ni `contrib`, ni `non-free`) : l'essai exigerait le dépôt d'Intel. **Abandonné** sauf besoin de capacité avéré.
- arm64 (Pi 5) : binaire ONNX Runtime précompilé disponible, performances **non mesurées**.

## Pièges relevés
- `ort` tire `native-tls` par défaut pour télécharger ses binaires au build : sans `libssl-dev`, utiliser `tls-rustls` ; le build nécessite un accès réseau à `cdn.pyke.io` (à prévoir pour la CI et le Dockerfile, ou pré-télécharger avec `ORT_LIB_LOCATION`).
- Les erreurs de `SessionBuilder` ne sont pas `Send` : ne pas les propager telles quelles dans `anyhow` (les convertir en chaîne).
