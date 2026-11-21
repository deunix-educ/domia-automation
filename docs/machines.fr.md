# Machines prises en charge

Ce qui a été testé, ce qui ne l'a pas été, et comment ajouter vos propres mesures. Rien n'est promis au-delà de la colonne « État ».

| Machine | État | Remarques |
|---|---|---|
| Mini-PC Intel Core i5-6500, 8 Go (référence) | **testée et mesurée** | 12 caméras en 1080p avec décodage matériel (VAAPI) : voir [Mesures](measures.md) |
| Mini-PC amd64, 16 Go (par exemple de la classe one-amd8) | **pas encore testée** | Même image que la référence. Attendue au moins aussi bonne ; le décodage matériel demande un GPU Intel avec VAAPI |
| Raspberry Pi 5, 8 Go | **pas encore testée** | Image arm64 jamais construite ni lancée à ce jour ; voir ci-dessous |
| Autre Linux arm64 ou amd64 avec Docker | non testée | L'installateur ne les refuse pas ; il avertit en arm64 |

## Raspberry Pi 5 : ce qu'on peut attendre, ce qu'on ignore

Connu par la conception (pas par des mesures) :

- **Système** : Raspberry Pi OS 64 bits (basé sur Debian 13) ou Debian 13 arm64 ; Docker avec le plugin Compose 2.24 ou plus récent. Un système 32 bits ne peut pas lancer les images.
- **Stockage** : utilisez un **SSD** (USB 3 ou NVMe), pas une carte SD. La base et les enregistrements vidéo écrivent en continu ; une carte SD s'use. L'installateur avertit quand `/` est sur une carte SD.
- **Décodage vidéo** : le décodeur matériel Intel (VAAPI) n'existe pas en arm64 ; la vidéo est décodée par le processeur, donc le nombre de caméras tenues est inférieur à celui de la machine de référence. Le chiffre est inconnu tant qu'il n'est pas mesuré.
- **Détection d'objets** (YOLO sur CPU) : non mesurée en arm64. Laissez-la désactivée au début, puis activez-la sur une caméra en surveillant la charge.
- **Première installation** : construire l'image sur le Pi est très long (comptez bien plus de 40 minutes) et demande de la mémoire ; préférez une image publiée ([ADR 0062](adr/0062-published-images-with-local-build-fallback.md)). Construire sur une autre machine arm64 puis publier est aussi possible.
- **Clé Zigbee** : fonctionne de la même façon (`/dev/ttyUSB0` ou `/dev/ttyACM0`).
- **Alimentation et refroidissement** : alimentation officielle et radiateur ou ventilateur ; `vcgencmd get_throttled` (repris par la fiche machine) montre si le Pi a déjà été bridé.

## Mesurer votre machine

Après `make prod-deploy` (ou `./install.sh`) et quelques minutes au repos :

```bash
tools/machine-card.sh > fiche.md         # ajoutez --with-backup pour chronométrer aussi une sauvegarde
```

La fiche liste le matériel, l'architecture de l'image, la mémoire et le processeur de chaque service au repos, la latence de l'interface (médiane et 95e centile sur 50 requêtes) et, sur un Pi, la température et le bridage. Elle est en lecture seule et ne contient aucun mot de passe. Ajoutez le temps de démarrage à froid (`make prod-down`, puis `time make prod-up`) et le nombre de caméras tenues, puis collez le résultat dans [Mesures](measures.md) ou partagez-le dans un ticket : un résultat sur une machine absente de ce tableau est la contribution la plus utile.

## Alléger pour une petite machine

- Moins de caméras, ou une résolution de flux plus basse côté caméra.
- `VIDEO_QUOTA_GB` et `IOT_QUOTA_GB` dans `.env` bornent l'historique conservé sur disque.
- Grafana et Prometheus peuvent être arrêtés (`docker compose stop grafana prometheus`) si la mémoire manque ; l'application continue, vous perdez les tableaux de bord.
- L'assistant local (un LLM) est un programme séparé et facultatif qui demande bien plus que ce qu'offre un Pi 5 ; n'y comptez pas.
