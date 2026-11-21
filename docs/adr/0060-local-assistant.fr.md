# ADR 0060 — Assistant local : moteur, modèles et limites


Statut : accepté, validé par l'utilisateur le 2026-11-17 (R12 lot A0). Mesures : `docs/bench/agent-bench-2026-11-17.md`.

## Contexte
R12 ajoute à l'interface un assistant en français qui appelle les outils du serveur MCP (R11). Les décisions du 2026-11-15 fixent l'architecture (agent dans `dominet`, LLM auto-hébergé sur une autre machine, `llama-server`, aucune API distante, rôle `Admin` seul, deux modèles : quotidien et configuration). Le lot A0 mesure ce que le matériel permet : la machine d'essai est le poste de dev (i9, 46 Go de RAM, RTX 4060 de 8 Go), qui a aussi servi de machine d'inférence pour l'essai.

## Options mesurées
Quatre modèles Qwen3 en Q4_K_M, avec le **vrai** prompt système et les **vrais** outils (profil quotidien : 29 outils, 4 930 tokens ; `confirm` et `dry_run` retirés), 30 demandes en français, température 0, sur GPU (couches réparties par `llama-server`) et sur CPU seul.

| Modèle | Mode | Génération (tok/s) | Carte d'action (s) | Outil juste | Remarque |
|---|---|---:|---:|---:|---|
| 4B Instruct 2507 | GPU | 67,7 | 0,4 | 76 % | trop de questions au lieu d'agir |
| **8B** | **GPU** | **16,2** | **1,8** | **96 %** | tient en entier dans 8 Go |
| 8B | CPU | 3,6 | 8,3 | 100 % | à la limite du seuil de 8 s |
| **14B** | GPU (partiel) | 3,9 | 7,4 | 92 % | déborde en RAM, lent mais sous 30 s |
| 14B | CPU | 1,9 | 15,1 | 96 % | |
| 30B-A3B Instruct 2507 | GPU | 21,2 | 1,2 | 80 % | **a inventé des appareils sans appeler d'outil** (spike raté) |
| Ministral-3 8B (Mistral) | GPU | 13,0 | 1,3 | 96 % | rapide, mais invente un nom de scène (« Tout éteindre ») sur « Éteins tout » ; réponse du spike non fidèle aux noms |
| Ministral-3 14B (Mistral) | GPU (partiel) | 4,8 | 3,3 | 88 % | contradiction dans le spike (« aucun appareil n'a rien émis… » puis la liste) ; 40 % de réussite sur les demandes ambiguës |
| Mistral-Small-3.2 24B | GPU (partiel) | 2,0 | 9,9 | 92 % | trop lent (au-dessus du seuil de 8 s), pas plus fiable que le Qwen3-14B |

Le préfixe (prompt système et outils) est mis en cache : un tour à chaud coûte 0,02 à 0,5 s de prefill.

Mesures Mistral : `docs/bench/agent-bench-2026-11-17-mistral.md`. Sur les demandes ambiguës (« Ferme-le », « Éteins tout »), les Mistral cherchent d'abord avec un outil de lecture au lieu de demander (le jeu les compte en échec, alors qu'une recherche est sans effet) ; les Qwen3 posent la question. Les mesures de vitesse des Mistral ont été légèrement perturbées par des vérifications du dépôt lancées en parallèle.

## Décision
- **Moteur** : `llama-server` (llama.cpp, CUDA), `--jinja` pour les appels d'outils, un modèle résident par instance, contexte 16 k.
- **Modèle quotidien** (`AGENT_MODEL`) : **Qwen3-8B Q4_K_M**. Carte d'action en moins de 2 s sur GPU de 8 Go, 96 % d'outils justes, questions posées sur toutes les demandes ambiguës.
- **Modèle configuration** (`AGENT_MODEL_ADMIN`) : initialement **Qwen3-14B Q4_K_M** ; **remplacé le 2026-11-17 par le même modèle que le quotidien** (voir « Décision de l'utilisateur » ci-dessous). Le réglage reste, vide par défaut (= `AGENT_MODEL`), pour un essai futur.
- **Mistral comparés et non retenus** (8B, 14B, Small 24B) : pas plus fiables que les Qwen3 de même taille, plus lents pour le 24B, et ils reformulent ou inventent des noms. Le 8B de Mistral est un repli sérieux du quotidien (96 %, 1,3 s).
- **Écartés** : le 4B (rate trop d'appels), le 30B-A3B (rapide mais il répond sans lire la maison : ce défaut, grave pour un assistant de diagnostic, rend le modèle inutilisable sans mesure plus fine).
- **Prompt système** : il doit dire d'**agir d'abord** (chercher avec les outils de lecture) et de ne poser une question que si la demande ne dit pas sur quoi agir ; la première version, trop prudente, faisait tomber le 4B à 52 %.
- **Sans GPU** : le 8B reste utilisable (8 s par carte, 3,6 tok/s) mais au seuil ; le 4B (6 s) est le repli.

## Limites de la mesure
30 demandes seulement : un écart de 4 points est un seul cas. Premier tour seulement (pas de dialogue en plusieurs étapes, hors un spike de 1 demande avec résultats d'outils simulés). Les écarts GPU/CPU d'un même modèle (le 4B passe de 76 % à 92 %) viennent de la numérique et montrent le bruit. Le jeu de 50 demandes (lot A5), les injections et les demandes de configuration longues restent à mesurer. La boucle d'essai est dans la référence ; la boucle Rust est le lot A1.

## Conséquences
- `make agent-bench` rejoue la mesure à chaque changement de machine ou de modèle ; le tableau est versionné dans `docs/bench/`.
- Le poids du modèle : 5 Go (8B), résident sur le GPU de 8 Go ; un seul `llama-server`.
- Qualité de l'assistant : elle dépendra autant des descriptions d'outils et du prompt que du modèle ; les 4 930 tokens d'outils sont un coût fixe, couvert par le cache.

## Évaluation A5 (2026-11-17)
Jeu de **50 demandes** jouées de bout en bout par la vraie boucle de l'agent (`make agent-eval`, `docs/bench/agent-eval-2026-11-17.md`), seuil **85 % et aucune carte pour une demande ambiguë ou une injection** : **Qwen3-8B 49/50 (98 %)**, **Qwen3-14B 48/50 (96 %)**, zéro carte à tort pour les deux. Le 14B n'est **pas plus fiable** que le 8B sur ce jeu (configuration 9/9 pour les deux) et **2 à 3 fois plus lent** (13,5 s contre 6,2 s par demande de configuration) ; il est meilleur sur les demandes ambiguës avant le rappel anti-promesse (8/8 contre 5/8). Le choix de deux modèles de l'ADR tient donc surtout à la prudence ; **un seul modèle (le 8B) suffit sur ces mesures**, décision de l'utilisateur ci-dessous. Le jeu est petit (un écart de 2 points = 1 cas) et la maison de test simple.

## Décision de l'utilisateur (2026-11-17)
**Un seul modèle : Qwen3-8B Q4_K_M**, pour tous les régimes. `AGENT_MODEL_ADMIN` est vide par défaut et signifie « le même que `AGENT_MODEL` » ; le 14B reste mesurable avec `make agent-eval` si les modèles évoluent. Conséquences : une seule instance `llama-server`, 5 Go à télécharger, demandes de configuration en 6 s environ.
