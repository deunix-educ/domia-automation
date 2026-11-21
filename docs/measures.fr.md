# Mesures

Tous les chiffres qui ont conduit une décision, avec la machine, la méthode et la façon de les refaire. À utiliser comme base de comparaison : un résultat loin de ces nombres signale un changement.

**Machines de référence.** *Cible* : Intel Core i5-6500 (Skylake, 4 cœurs, sans AVX-512), 7,8 Go de RAM, Intel HD 530 (VAAPI), Debian 13. *Développement* : 20 fils, 48 Go de RAM, RTX 4060 8 Go, pas de VAAPI utilisable.

## 1. Spikes de faisabilité (phase R0)

| Sujet | Verdict | Chiffre clé |
|---|---|---|
| Leptos en rendu serveur avec hydratation, WebSocket et traductions | go | 16 contrôles navigateur sur 16 ; WASM 448 Ko (190 Ko gzip) |
| Stockage de séries temporelles, 1 million d'enregistrements | go | 1 680 écritures/s ; requête à seuil 99 ms par appareil |
| Live WHEP en WASM | go | latence 40 ms (VP8, boucle locale) ; live 720p réel validé |
| Décodage H.264 | go, avec `ffmpeg` | 4 caméras : 22 % de la machine en logiciel, 8 % avec VAAPI |
| ONNX Runtime avec YOLOv8n | go avec réserves | 43 à 106 ms par image ; détections identiques au modèle de référence sur 10 images sur 10 |
| ONVIF sur 4 caméras réelles | go | authentification, profils, PTZ et coupure infrarouge fonctionnent ; une caméra absente de la découverte multicast |
| Scripts Rhai isolés | go | 18 sur 18 ; un script hostile est coupé en moins de 45 ms |
| Client MQTT | go | aucune perte à 2 000 msg/s ; reconnexion en 0,6 s |

## 2. Charge vidéo (phase R6)

Outil : `tools/bench-video.py`. Rien n'est partagé avec un système en service : PostgreSQL, MediaMTX et ReductStore jetables sur des ports décalés, aucune caméra réelle.

**Protocole.** Les caméras simulées sont un même clip H.264 1080p, 15 images/s, 3 Mbit/s, encodé une fois et republié en `-c copy` (leur coût propre est de 0,01 à 0,08 cœur au total). Trois scénarios, 20 s de chauffe puis 60 s de mesure, un échantillon toutes les 5 s : *décodage seul* (2 images décodées par seconde) ; *complet* (enregistrement continu en segments fMP4 de 60 s, indexation et archivage, mouvement toutes les 30 s, YOLO à chaque mouvement) ; *stress* (mouvement toutes les 5 s, six fois plus de YOLO). « Cœurs » compte l'application, ses `ffmpeg` de décodage, MediaMTX, ReductStore et PostgreSQL ensemble, en cœurs entiers moyens.

| Machine cible | Caméras | Cœurs | Mémoire (Mo) | Machine occupée |
|---|---|---|---|---|
| VAAPI, décodage seul | 4 | 0,46 | 673 | 15 % |
| VAAPI, décodage seul | 12 | 1,35 | 1 782 | 45 % |
| VAAPI, complet | 4 | 0,55 | 863 | 18 % |
| VAAPI, complet | 12 | 1,47 | 1 885 | 45 % |
| VAAPI, stress | 12 | 1,34 | 1 826 | 43 % |
| Logiciel, décodage seul | 4 | 1,12 | 747 | 30 % |
| Logiciel, complet | 8 | 2,20 | 1 445 | 60 % |

Ce qu'on en tire :

- Une caméra coûte environ **0,11 cœur** avec VAAPI (0,12 en mode complet) et **0,28 cœur** en logiciel. VAAPI divise le coût de la vidéo par environ trois (`ffmpeg` de décodage : 0,31 contre 0,98 cœur pour 4 caméras ; sur un fichier, 3,5 s de processeur contre 10,4 s pour 100 s de vidéo).
- **12 caméras en 1080p tiennent à 45 % de la machine cible avec VAAPI** (1,5 cœur, 1,9 Go), 8 en logiciel (60 %). Le coût est linéaire : la moitié de la machine reste libre pour le reste.
- YOLO n'est pas le facteur limitant : l'application reste sous 0,6 cœur même en stress à 12 caméras (environ 105 ms par image sur un cœur).
- La mémoire est d'environ 150 Mo par caméra plus environ 0,3 Go de base (PostgreSQL, ReductStore, l'application).
- Sur la machine réelle, une fois VAAPI activé dans le conteneur, le processeur de l'application est passé de 85-90 % d'un cœur à environ 36 % (41 à 49 % pendant la surveillance de 48 h avec tous les services).

Limites : flux synthétiques identiques (des flux réels peuvent coûter plus : 5 mégapixels, certains profils H.265 non décodés par le VAAPI de Skylake, débit variable) ; aucun trafic web, Zigbee, Grafana ou MQTT pendant la mesure ; le live WebRTC vers les navigateurs n'est pas mesuré (il dépend du nombre de spectateurs ; MediaMTX ne ré-encode pas).

Pour refaire la mesure :
```
make fetch-model && cargo leptos build --release
python3 tools/bench-video.py --bin target/release/dominet --site target/site --model models/yolov8n.onnx \
  --bus crates/dominet-surveillance/fixtures/yolo/bus.jpg --workdir /tmp/bench --counts 1,4,8,12 --seconds 60
```
Utiliser `--nice 19` sur une machine partagée et `FFMPEG_HWACCEL=off` pour la série logicielle.

## 3. Charge MQTT et WebSocket (phase R8)

Mesurée sur le poste de développement, jamais sur la production.

- **MQTT** : `cargo test -p dominet-server --test throughput` : 50 appareils, 2 000 messages/s soutenus pendant 5 s contre le Mosquitto de développement, aucun état final perdu, publication refusée proprement (jamais bloquée) quand le client est saturé. 7 s.
- **WebSocket** : `cargo test --release -p dominet --test ws_load -- --nocapture` (variables `WS_LOAD_USERS`, `WS_LOAD_RATE`, `WS_LOAD_MESSAGES`, `WS_LOAD_P99_MS`).

| Clients | Cadence | États livrés | Latence p50 / p99 / max | Mémoire du processus |
|---|---|---|---|---|
| 200 (25 comptes × 8) | 200 /s | 120 000 / 120 000 | 4 / 13 / 15 ms | 15 à 78 Mo |
| 1 000 (125 comptes × 8) | 100 /s | 600 000 / 600 000 | 79 / 108 / 128 ms | 15 à 315 Mo |

La mémoire comprend les 1 000 clients de test, qui vivent dans le même processus. Une compilation de débogage est environ 100 fois plus lente (p99 de 1,1 s avec 200 clients) : la borne par défaut du test en tient compte. Un foyer compte quelques navigateurs ouverts, très en dessous de ces charges. Mosquitto jette les messages au-delà de 1 000 en attente par client si `max_queued_messages` n'est pas relevé (voir la page des pièges).

## 4. Surveillance de la production (phase R8, 48 heures)

13 services sains, aucun redémarrage automatique, aucune alerte, toutes les sauvegardes présentes. Mémoire de 2,6 à 2,9 Go, charge de 1,3 à 3, base de 58 Mo croissant d'environ 60 à 70 Mo par jour (environ 3 relevés par seconde). Les automations d'événements solaires se sont exécutées à l'heure calculée avec les coordonnées du site. À l'époque : 863 tests Rust et 29 scripts E2E (783 vérifications) sur l'image.

## 5. Assistant local (phase R12)

Machine : développement, llama.cpp b12078 avec CUDA 12, contexte 16 384, température 0, réflexion désactivée. Seuils : une carte d'action quotidienne en moins de 8 s, une étape de configuration en moins de 30 s.

| Modèle | Mode | Génération (tok/s) | Carte d'action, médiane (s) | Bon outil |
|---|---|---|---|---|
| Qwen3-8B Q4_K_M | GPU | 16,2 | 1,8 | 96 % |
| Qwen3-8B Q4_K_M | CPU | 3,6 | 8,3 | 100 % |
| Qwen3-14B Q4_K_M | GPU | 3,9 | 7,4 | 92 % |
| Qwen3-4B Instruct Q4_K_M | GPU | 67,7 | 0,4 | 76 % |
| Qwen3-30B-A3B Instruct Q4_K_M | GPU | 21,2 | 1,2 | 80 % (a échoué au spike) |

Évaluation de bout en bout, 50 demandes jouées dans la vraie boucle de l'agent avec les vrais outils contre une base de test à trois noms d'appareils volontairement piégeux : **Qwen3-8B 49 sur 50 (98 %) en 237 s, Qwen3-14B 48 sur 50 (96 %) en 783 s, aucune mauvaise carte pour l'un comme pour l'autre** (seuil : 85 % et zéro mauvaise carte). Le 8B est environ trois fois plus rapide à qualité égale, un seul modèle a donc été gardé. Le jeu compte maintenant 51 demandes (49 réussies). Il se joue avec `cargo test -p dominet-agent --test eval -- --ignored` contre un `llama-server` en marche (variables `AGENT_LIVE_URL`, `AGENT_LIVE_MODEL`, `AGENT_EVAL_OUT`) ; les résultats bruts sont dans `docs/bench/`.

## 6. Durées de construction et de test

- `make ci` (format, lints, tests, WASM, deny, audit) : environ 4 min à chaud ; le hook `pre-push` le rejoue sur une copie propre du commit poussé en environ 16 min (956 à 1 029 s sur les derniers pushes).
- Les tests sont bornés à 8 fils ; à 20 fils, `PoolTimedOut` apparaît environ une fois sur trois.
- WASM release d'environ 3 Mo ; WASM de développement 48 Mo. Image d'environ 713 Mo.
- Disque sur la cible : la croissance propre à l'application est faible (environ 65 Mo par jour d'historique) ; la vidéo domine, dimensionner d'abord le volume des enregistrements.
