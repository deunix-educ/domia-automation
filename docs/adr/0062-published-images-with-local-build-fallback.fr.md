# ADR 0062 — Images multi-architecture publiées, avec repli sur la construction locale

Statut : accepté (P3 lot 2, décisions de l'utilisateur, 2026-11-21).

## Contexte
Construire l'image prend de 15 à 40 minutes (Rust, WebAssembly, moteur YOLO) et demande plusieurs Go de mémoire ; sur un Raspberry Pi 5, c'est le principal obstacle d'un débutant. Les machines visées sont x86-64 (mini-PC) et arm64 (Raspberry Pi 5). Le dépôt n'est pas encore public : une image ne peut pas être téléchargée avant la publication (P6).

## Options
- **A. Toujours construire en local.** Simple et hors ligne, mais lent et fragile sur les petites machines.
- **B. Image publiée seulement.** Rapide, mais l'installation échoue si le registre ou l'étiquette est inaccessible, et rien ne s'essaie avant la publication.
- **C. Essayer l'image publiée, se rabattre sur la construction locale.** Rapide quand elle existe, fonctionne hors ligne, depuis un fork ou avant la publication.

## Décision
Option C. `tools/prod.sh pull-image` (appelé par `make prod-up`, donc par `./install.sh` et `make prod-update`) télécharge `ghcr.io/deunix-educ/domia-automation:latest`, l'étiquette `dominet:<DOMINET_TAG>` (le nom que Compose utilise déjà : les fichiers Compose ne changent pas) et l'appelant construit en local si le téléchargement échoue. `.env` peut changer la source (`DOMINET_REGISTRY_IMAGE`, `DOMINET_IMAGE_VERSION`) ou forcer la construction (`DOMINET_PULL=0`). Les images sont publiées par `tools/publish-image.sh` (`docker buildx`, `linux/amd64` et `linux/arm64`), lancé à la main par le mainteneur ; il n'y a pas de CI automatique (ADR 0015).

## Conséquences
- Jusqu'à la première publication, toute installation se rabat sans bruit sur la construction (quelques secondes perdues sur le téléchargement raté, avec un message qui le dit).
- Une image téléchargée est crue sur parole : elle n'est pas comparée au code source de la machine. Qui le souhaite fixe `DOMINET_PULL=0`.
- L'image arm64 n'a encore jamais été construite ; sa première construction fait partie de l'essai sur Raspberry Pi 5 (P4).
- `latest` bouge à chaque publication ; fixer `DOMINET_IMAGE_VERSION` sur une étiquette de version évite les surprises à `make prod-update`.
