# Bench de l'assistant local : machine de développement, 2026-11-17

Machine : i9, 46 Go RAM, RTX 4060 8 Go (poste de dev). Moteur : llama.cpp b12078 (CUDA 12, prebuilt). Contexte 16384. Temperature 0, reflexion desactivee.

## Synthese

| Modele | Mode | Prompt (tok) | Prefill a froid (tok/s) | Prefill a chaud (s) | Generation (tok/s) | Carte d'action (s, mediane) | Diagnostic 300 tok (s) | Outil juste | Appel complet | Question si ambigu | Spike |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|:--:|
| Ministral-3-14B-Instruct-2512-Q4_K_M.gguf | gpu | 4745 | 498 | 0.21 | 4.8 | 3.3 | 63 | 88 % | 88 % | 40 % | ok |
| Ministral-3-14B-Instruct-2512-Q4_K_M.gguf | cpu | 4745 | 296 | 0.43 | 2.3 | 7.1 | 129 | 92 % | 92 % | 40 % | ok |
| Ministral-3-8B-Instruct-2512-Q4_K_M.gguf | gpu | 4745 | 1212 | 0.08 | 13.0 | 1.3 | 23 | 96 % | 96 % | 80 % | ECHEC |
| Ministral-3-8B-Instruct-2512-Q4_K_M.gguf | cpu | 4745 | 465 | 0.28 | 3.6 | 4.8 | 83 | 96 % | 96 % | 80 % | ECHEC |
| Mistral-Small-3.2-24B-Instruct-2506-Q4_K_M.gguf | gpu | 4745 | 224 | 0.49 | 2.0 | 9.9 | 149 | 92 % | 92 % | 40 % | ok |
| Mistral-Small-3.2-24B-Instruct-2506-Q4_K_M.gguf | cpu | 4745 | 172 | 0.66 | 1.5 | 13.3 | 199 | 92 % | 92 % | 40 % | ECHEC |

Seuils (ROADMAP-agent.md §7) : carte d'action quotidienne < 8 s, etape de configuration < 30 s.

## Ministral-3-14B-Instruct-2512-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 23.0 s, etapes : devices_list.

> Aucun appareil n'a rien émis depuis 24 heures. Les appareils les plus silencieux sont :
- **Temperature-CH-sud_est-haut** (Chambre) : 87 heures sans émission.
- **Relais-lora-1** (Serre) : 41 heures sans émission.

Souhaites-tu vérifier leur état ou agir sur un autre appareil ?

Echecs :

- #15 « Ouvre le volet Volet-salon à 40 %. » -> device_get {"device": "Volet-salon"}
- #18 « Affiche un message à tout le monde : le dîner est prêt. » -> None {}
- #21 « Ferme-le. » -> devices_list {"name_contains": "volet"}
- #22 « Fais quelque chose pour la lumière. » -> devices_list {"device_type": "light", "limit": 5}
- #23 « Éteins tout. » -> devices_list {"include_inactive": false}
- #29 « Active l'appairage Zigbee. » -> None {}

## Ministral-3-14B-Instruct-2512-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 45.5 s, etapes : devices_list.

> Aucun appareil n'a rien émis depuis 24 heures. Les appareils les plus silencieux sont :
- **Temperature-CH-sud_est-haut** (Chambre) : 87 heures sans communication.
- **Relais-lora-1** (Serre) : 41 heures sans communication.

Souhaites-tu agir sur l'un d'eux ?

Echecs :

- #15 « Ouvre le volet Volet-salon à 40 %. » -> device_get {"device": "Volet-salon"}
- #21 « Ferme-le. » -> devices_list {"name_contains": "volet"}
- #22 « Fais quelque chose pour la lumière. » -> scenes_list {}
- #23 « Éteins tout. » -> devices_list {"include_inactive": false}
- #29 « Active l'appairage Zigbee. » -> None {}

## Ministral-3-8B-Instruct-2512-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : rate en 8.0 s, etapes : devices_list.

> Voici les appareils qui n'ont pas émis de données depuis 24 heures :
- **Capteur de température de la Chambre** (silencieux depuis 87 heures)
- **Relais LoRa dans la Serre** (silencieux depuis 41 heures).

Souhaites-tu vérifier leur état ou agir sur l'un d'eux ?

Echecs :

- #23 « Éteins tout. » -> scene_run {"scene": "Tout éteindre"}
- #25 « Quels types de configuration puis-je modifier ? » -> None {}

## Ministral-3-8B-Instruct-2512-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : rate en 29.2 s, etapes : devices_list.

> Voici les appareils qui n'ont pas émis de données depuis 24 heures :
- **Capteur de température** dans la chambre (dernière donnée il y a 87 heures).
- **Relais LoRa** dans la serre (dernière donnée il y a 41 heures).

Souhaites-tu vérifier leur état ou agir sur l'un d'eux ?

Echecs :

- #23 « Éteins tout. » -> scene_run {"scene": "Toutes les lumières"}
- #25 « Quels types de configuration puis-je modifier ? » -> None {}

## Mistral-Small-3.2-24B-Instruct-2506-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 27.1 s, etapes : devices_list.

> Deux appareils n'ont rien émis depuis 24 h : Temperature-CH-sud_est-haut et Relais-lora-1.

Echecs :

- #21 « Ferme-le. » -> devices_list {"name_contains": "volet"}
- #22 « Fais quelque chose pour la lumière. » -> devices_list {"name_contains": "lumière"}
- #23 « Éteins tout. » -> devices_list {"include_inactive": true}
- #26 « Liste mes emplacements. » -> None {}
- #27 « Crée un emplacement nommé Cave. » -> None {}

## Mistral-Small-3.2-24B-Instruct-2506-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : rate en 33.2 s, etapes : devices_list.

> Deux appareils n'ont rien émis depuis longtemps : le capteur de température de la chambre et le relais de la serre.

Echecs :

- #21 « Ferme-le. » -> devices_list {"name_contains": "volet"}
- #22 « Fais quelque chose pour la lumière. » -> devices_list {"name_contains": "lumière"}
- #23 « Éteins tout. » -> devices_list {"include_inactive": true}
- #26 « Liste mes emplacements. » -> None {}
- #27 « Crée un emplacement nommé Cave. » -> None {}
