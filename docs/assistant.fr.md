# Assistant local (R12)

L'**assistant** répond en français, dans l'interface, à des demandes comme « Quels volets sont encore ouverts ? » ou « Ferme-les ». Il tourne **chez vous** : le modèle est un `llama-server` auto-hébergé, aucune API distante, aucune donnée ne sort du réseau. Feuille de route : `ROADMAP-agent.md` ; décision et mesures : [ADR 0060](adr/0060-local-assistant.md) ; évaluation : [50 demandes](bench/agent-eval-2026-11-17.md).

!!! info "Réservé aux administrateurs, désactivé par défaut"
    La page **Administration › Assistant** (`/admin/assistant`) est réservée au rôle administrateur ; elle ne répond que si `AGENT_ENABLED` est vrai.

## Ce qu'il fait, et ce qu'il ne fait jamais seul
- **Lire** : état des appareils, historique, journaux, alertes, configuration (jamais un secret).
- **Agir** : le modèle **propose** une action, vous la voyez dans une **carte** (« Fermer le volet du salon »), puis vous cliquez **Confirmer** ou **Annuler**. Rien ne s'exécute sans votre clic.
- **Configurer** (type de demande « configuration ») : pour une automation, la carte montre en plus le YAML **avant / après** en différences.
- **Annuler** : chaque modification confirmée est enregistrée dans le journal (type `agent`) avec votre demande, l'état d'avant et le moyen de revenir en arrière ; la liste « Modifications faites par l'assistant » permet d'annuler en deux clics. Ne sont pas annulables : commandes d'appareils, scènes lancées, notifications, et tout ce qui touche à du code Rhai.

## Installer le modèle
Sur la machine d'inférence (un PC avec carte graphique de 8 Go convient ; sans GPU c'est possible mais environ 8 s par réponse), avec un `llama-server` de llama.cpp déjà installé :
```bash
make agent-install LLAMA_DIR=/chemin/vers/llama.cpp  AGENT_HOST=0.0.0.0
make agent-install LLAMA_DIR=... DRY=1     # montre ce qui serait fait, sans rien faire
```
Le script télécharge **Qwen3-8B Q4_K_M** (5 Go, repris si interrompu) dans `~/IA/models` et crée le service utilisateur `dominet-llm` (contexte 16 k, appels d'outils, une demande à la fois). Pour qu'il survive à la déconnexion : `sudo loginctl enable-linger $USER`. Un **seul modèle** sert tous les types de demandes (ADR 0060, décision du 2026-11-17).

!!! warning "Réseau"
    `llama-server` n'a pas d'authentification. Ne l'ouvrez qu'au réseau local ou au VPN (`AGENT_HOST` = adresse du LAN), jamais à Internet.

## Activer côté DomiNet
Dans **Administration › Réglages**, catégorie **Assistant** :

| Réglage | Valeur |
|---|---|
| `AGENT_ENABLED` | `true` |
| `AGENT_LLM_URL` | `http://<machine d'inférence>:8081` (port par défaut de `make agent-install` ; le 8080 reste à Zigbee2MQTT et à la démo) |
| `AGENT_MODEL` | `Qwen3-8B-Q4_K_M` |
| `AGENT_MODEL_ADMIN` | vide (même modèle) |

Sur la page Assistant, le bouton **Tester la connexion** vérifie l'adresse et le modèle. Si la machine d'inférence est éteinte, la page l'indique et les formulaires restent utilisables.

## Premiers essais
1. « Quelles lampes sont allumées ? » → une réponse en une ou deux phrases.
2. « Ferme le volet du salon » → une carte ; **Confirmer**.
3. Type « configuration » : « Ferme les volets au coucher du soleil » → une carte d'automation avec le YAML avant / après ; **Annuler** si elle ne convient pas.
4. Menu **Modifications faites par l'assistant** : annuler la dernière.

## Limites
- Un petit modèle peut se tromper ou demander une précision : reformulez avec le nom de l'appareil ou de la pièce.
- Une demande à la fois, 40 messages de mémoire, 15 minutes d'inactivité avant l'oubli de la conversation ; la conversation n'est pas conservée en base.
- Ce que la maison contient (noms d'appareils, messages) est traité comme une donnée, jamais comme une consigne : un appareil nommé « ignore tes instructions » est sans effet (essais d'injection : 0 carte à tort).
- Modifier ou renommer un élément qui existe (un appareil par exemple) n'est pas fiable avec un petit modèle : faites-le depuis l'administration. L'assistant sait lire, piloter et créer.
- Qualité mesurée : 49 demandes sur 51 réussies avec Qwen3-8B (les deux échecs : « Éteins tout » et « Mets la bonne température » sans question de précision, sans carte), 0 carte proposée à tort. Rejouer la mesure : `make agent-eval LLAMA_DIR=… AGENT_EVAL_MODELS=Qwen3-8B-Q4_K_M`.
