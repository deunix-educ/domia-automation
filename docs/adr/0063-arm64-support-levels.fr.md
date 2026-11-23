# ADR 0063 — arm64 et Raspberry Pi 5 : pris en charge au mieux, avec un état de test explicite

Statut : accepté (P4, 2026-11-21). Rouvre le point « arm64 non construit » de l'[ADR 0017](0017-docker-image-and-e2e.md).

## Contexte
La première cible est x86-64 (mini-PC Intel Core i5). Un Raspberry Pi 5 (8 Go) est la machine la moins chère qu'un débutant puisse acheter. La machine de développement n'a ni émulation arm64 ni matériel arm64 : rien n'a été construit ni lancé en arm64. Faits connus : ONNX Runtime fournit un binaire arm64 précompilé ([ADR 0009](0009-yolo-ort.md)) ; les paquets Debian `intel-media-va-driver` et `vainfo` n'existent qu'en amd64, ce qui cassait une construction arm64 du `Dockerfile` ; le Pi n'a pas de décodeur VAAPI utilisable par `ffmpeg` dans l'image.

## Options
- **A. Déclarer arm64 non pris en charge.** Honnête aujourd'hui, mais ferme la porte sur la machine que la plupart des gens possèdent.
- **B. Déclarer arm64 pris en charge.** Promet ce qui n'a pas été vérifié.
- **C. Rendre l'image constructible en arm64, écrire « non testé » dans le tableau des machines tant qu'il n'existe pas de mesure, et donner l'outil pour la produire.**

## Décision
Option C.
- Le `Dockerfile` installe les paquets VAAPI Intel en amd64 seulement ; en arm64 la vidéo est décodée par le processeur. `docker buildx build --check --platform linux/arm64` passe ; aucune construction arm64 n'a été lancée.
- `docs/machines.md` donne un état par machine (« testée et mesurée », « pas encore testée », « non testée ») et liste ce qui est connu et inconnu pour le Pi.
- L'installateur et `preflight` ne refusent plus arm64 : ils avertissent, et avertissent de nouveau sur une carte SD ou sous 7 Go de mémoire.
- `tools/machine-card.sh` produit une fiche Markdown en lecture seule (matériel, architecture de l'image, mémoire et processeur par service, latence de l'interface, température et bridage du Pi) à coller dans `docs/measures.md`.
- Une machine passe à « testée » seulement avec une fiche et une installation à froid faites sur elle.

## Conséquences
- La première construction arm64 peut révéler d'autres blocages (un outil téléchargé sans binaire arm64, la mémoire à l'édition de liens WebAssembly) ; ils seront corrigés alors, pas devinés maintenant.
- Le nombre de caméras sur un Pi est inconnu et probablement bien inférieur à celui de la machine de référence ; rien dans la documentation n'en promet un.
- La publication d'une image arm64 (ADR 0062) devrait attendre une construction native et un démarrage réussis sur un Pi 5.

## Résultat (2026-11-23)
La condition est remplie sur un Raspberry Pi 5 (8 Go, NVMe, Debian 13) : construction native en 67 minutes, pile démarrée, interface qui répond. L'image arm64 a été fusionnée avec l'amd64 en `:1.0.1` et `:latest` multi-architecture (les sources compilées n'avaient pas changé depuis la 1.0.1). Le premier essai a révélé un défaut sans rapport avec la construction : en arm64, `docker pull` téléchargeait l'image amd64 sans erreur et elle remplaçait la construction locale ; `prod.sh` ne télécharge plus que l'architecture de la machine. Le nombre de caméras et la charge de la détection restent à mesurer.
