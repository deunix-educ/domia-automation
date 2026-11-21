# Bench de l'assistant local : machine de développement, 2026-11-17

Machine : i9, 46 Go RAM, RTX 4060 8 Go (poste de dev). Moteur : llama.cpp b12078 (CUDA 12, prebuilt). Contexte 16384. Temperature 0, reflexion desactivee.

## Synthese

| Modele | Mode | Prompt (tok) | Prefill a froid (tok/s) | Prefill a chaud (s) | Generation (tok/s) | Carte d'action (s, mediane) | Diagnostic 300 tok (s) | Outil juste | Appel complet | Question si ambigu | Spike |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|:--:|
| Qwen3-14B-Q4_K_M.gguf | gpu | 4933 | 444 | 0.25 | 3.9 | 7.4 | 78 | 92 % | 92 % | 100 % | ok |
| Qwen3-14B-Q4_K_M.gguf | cpu | 4933 | 276 | 0.53 | 1.9 | 15.1 | 160 | 96 % | 96 % | 100 % | ok |
| Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf | gpu | 4929 | 246 | 0.05 | 21.2 | 1.2 | 14 | 80 % | 80 % | 100 % | ECHEC |
| Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf | cpu | 4929 | 187 | 0.16 | 6.0 | 4.7 | 50 | 80 % | 80 % | 100 % | ECHEC |
| Qwen3-4B-Instruct-2507-Q4_K_M.gguf | gpu | 4929 | 3622 | 0.02 | 67.7 | 0.4 | 4 | 76 % | 76 % | 100 % | ok |
| Qwen3-4B-Instruct-2507-Q4_K_M.gguf | cpu | 4929 | 811 | 0.20 | 4.9 | 6.1 | 61 | 92 % | 92 % | 100 % | ok |
| Qwen3-8B-Q4_K_M.gguf | gpu | 4933 | 1397 | 0.06 | 16.2 | 1.8 | 19 | 96 % | 96 % | 100 % | ok |
| Qwen3-8B-Q4_K_M.gguf | cpu | 4933 | 495 | 0.29 | 3.6 | 8.3 | 83 | 100 % | 100 % | 100 % | ok |

Seuils (ROADMAP-agent.md §7) : carte d'action quotidienne < 8 s, etape de configuration < 30 s.

## Qwen3-14B-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 20.9 s, etapes : devices_list.

> Deux appareils n'ont rien émis depuis plus de 24 heures : "Temperature-CH-sud_est-haut" dans la Chambre et "Relais-lora-1" dans la Serre.

Echecs :

- #4 « Quelle est la température du bureau depuis ce matin ? » -> None {}
- #8 « Combien ai-je consommé d'électricité ces 7 derniers jours ? » -> None {}

## Qwen3-14B-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 42.2 s, etapes : devices_list.

> Deux appareils n'ont rien émis depuis plus de 24 heures : "Temperature-CH-sud_est-haut" dans la Chambre et "Relais-lora-1" dans la Serre.

Echecs :

- #4 « Quelle est la température du bureau depuis ce matin ? » -> None {}

## Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : rate en 2.8 s, etapes : aucune.

> Voici la liste des appareils qui n'ont pas envoyé de données depuis 24 heures : volet du salon, lampe de la cuisine, prise du bureau, et capteur de température du couloir.

Echecs :

- #7 « Quelles scènes existent ? » -> None {}
- #8 « Combien ai-je consommé d'électricité ces 7 derniers jours ? » -> None {}
- #14 « Allume la lampe du bureau. » -> None {}
- #16 « Lance la scène Cinéma. » -> None {}
- #25 « Quels types de configuration puis-je modifier ? » -> None {}

## Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : rate en 13.9 s, etapes : devices_list.

> Deux appareils n'ont rien émis depuis 24 heures : le capteur de température dans la chambre et le relais de la serre.

Echecs :

- #7 « Quelles scènes existent ? » -> None {}
- #8 « Combien ai-je consommé d'électricité ces 7 derniers jours ? » -> None {}
- #14 « Allume la lampe du bureau. » -> None {}
- #16 « Lance la scène Cinéma. » -> None {}
- #25 « Quels types de configuration puis-je modifier ? » -> None {}

## Qwen3-4B-Instruct-2507-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 1.2 s, etapes : devices_list.

> Les appareils qui n'ont rien émis depuis 24 h sont : Temperature-CH-sud_est-haut et Relais-lora-1.

Echecs :

- #4 « Quelle est la température du bureau depuis ce matin ? » -> None {}
- #5 « Pourquoi l'automation « Ouvrir le volet » a-t-elle échoué ? » -> None {}
- #9 « Y a-t-il eu des détections de personne sur la caméra du portail ? » -> None {}
- #20 « Tourne la caméra de la terrasse vers la gauche. » -> None {}
- #25 « Quels types de configuration puis-je modifier ? » -> None {}
- #28 « Je veux fermer les volets au coucher du soleil, par une automation. » -> None {}

## Qwen3-4B-Instruct-2507-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 14.1 s, etapes : devices_list.

> Les appareils qui n'ont rien émis depuis 24 h sont : Temperature-CH-sud_est-haut et Relais-lora-1.

Echecs :

- #25 « Quels types de configuration puis-je modifier ? » -> None {}
- #28 « Je veux fermer les volets au coucher du soleil, par une automation. » -> None {}

## Qwen3-8B-Q4_K_M.gguf (gpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 6.5 s, etapes : devices_list.

> Les appareils qui n'ont rien émis depuis 24 heures sont :  
- Temperature-CH-sud_est-haut (dans la Chambre)  
- Relais-lora-1 (dans la Serre)  

Voulez-vous en savoir plus sur l'un d'eux ?

Echecs :

- #25 « Quels types de configuration puis-je modifier ? » -> None {}

## Qwen3-8B-Q4_K_M.gguf (cpu)

Spike « Quels appareils n'ont rien émis depuis 24 h ? » : reussi en 27.7 s, etapes : devices_list.

> Les appareils qui n'ont rien émis depuis 24 heures sont :  
- Temperature-CH-sud_est-haut (dans la Chambre)  
- Relais-lora-1 (dans la Serre)  

Voulez-vous en savoir plus sur l'un d'eux ?

Echecs :

- aucun
