# Installation rapide

L'assistant d'installation vérifie votre machine, pose quatre questions et déploie l'application. Il demande quelques minutes de votre temps ; le premier démarrage peut être plus long, le temps de construire ou de télécharger les images.

## Ce qu'il vous faut

- Une machine qui reste allumée à la maison : mini-PC, petit serveur ou Raspberry Pi 5 (voir [Machines prises en charge](machines.md) et [Mesures](measures.md) pour ce que chacune tient). Linux, avec **Docker** et le plugin **Docker Compose** (version 2.24 ou plus récente), ainsi que `git`, `make`, `openssl` et `curl`.
- Au moins 2 Go de mémoire (4 Go ou plus recommandés) et 10 Go d'espace disque libre (les enregistrements vidéo en demandent bien plus).
- Votre utilisateur doit pouvoir lancer `docker` (membre du groupe `docker`).

## Installer

```bash
git clone https://github.com/deunix-educ/domia-automation.git
cd domia-automation
./install.sh
```

L'assistant déroule quatre étapes et s'arrête avec un message clair si quelque chose bloque :

1. **Vérifie la machine** : système, versions de Docker et de Compose, mémoire, espace disque, ports libres (80, 443, 1883), architecture.
2. **Pose quelques questions** : l'adresse que les appareils utiliseront (par défaut `<nom de la machine>.local`), le nom et le mot de passe du compte administrateur (masqué, 10 caractères au moins), une adresse e-mail pour les alertes (facultative). Il détecte seul une clé Zigbee et le décodage vidéo matériel.
3. **Écrit la configuration** (`.env`, lisible par vous seul) : tous les mots de passe et clés sont générés pour vous et jamais affichés.
4. **Déploie** : certificats, construction ou téléchargement, démarrage, compte administrateur, premières données, vérification finale.

## Essayer d'abord

```bash
./install.sh --dry-run
```

lance les vérifications et les questions et **ne change rien**. `./install.sh --help` liste les options (`--yes` pour un usage sans question, `--domain`, `--admin`, `--email`, `--no-zigbee`, `--reuse`, `--lang fr|en`).

## Après l'installation

1. Ouvrez `https://<adresse>` et connectez-vous avec le compte administrateur.
2. Sur chaque téléphone et ordinateur, installez le certificat une fois en ouvrant `http://<adresse>/dominet-ca.crt` (l'application utilise un certificat signé par votre propre autorité ; la page explique pourquoi les navigateurs posent la question).
3. **Copiez `DOMINET_MASTER_KEY` de `.env` dans un endroit sûr, loin de la machine.** Sans elle, une sauvegarde restaurée perd les secrets chiffrés (mots de passe des caméras).
4. Ajoutez vos appareils et vos caméras ; le reste de cette documentation décrit chaque page.

## Images

L'assistant essaie d'abord une image toute faite (amd64 et arm64) du registre du projet, ce qui économise de 15 à 40 minutes de compilation ; si elle n'est pas disponible (hors ligne, avant la publication, un fork) il construit l'image sur la machine. `DOMINET_PULL=0` dans `.env` force la construction ; `DOMINET_IMAGE_VERSION` fixe une version. Voir l'[ADR 0062](adr/0062-published-images-with-local-build-fallback.md).

## En cas de problème

- L'assistant nomme le point bloquant et comment le corriger (par exemple « le port 443 est déjà utilisé » : arrêtez ce qui l'utilise).
- Il n'écrase jamais un `.env` existant. Pour refaire un déploiement échoué avec la même configuration : `./install.sh --reuse`.
- Pour suivre les messages de l'application : `make prod-logs S=dominet N=100`.
- Le diagnostic, en lecture seule, qui dit quoi corriger : `make prod-doctor` (voir [Dépannage](troubleshooting.md)).
- Avant d'installer : `make preflight`.

Pour la procédure manuelle, toutes les options et la mise à jour, voir [Déploiement en production](deployment.fr.md).
