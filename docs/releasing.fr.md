# Publier une version et passer en public

Marche à suivre du mainteneur. Rien n'est automatique : il n'y a pas de service de CI (ADR 0015), la barrière est `make ci` lancé en local. **Le dépôt reste privé jusqu'à un feu vert explicite du propriétaire.**

## 1. Avant la première publication

1. **Décider de l'historique.** Le dépôt public peut commencer par un seul commit (« Initial public release ») pour que l'historique de travail n'apparaisse pas. Faites-le sur une *copie* et comparez les arbres :
   ```bash
   tree=$(git rev-parse 'HEAD^{tree}')
   GIT_AUTHOR_NAME=... GIT_AUTHOR_EMAIL=... GIT_COMMITTER_NAME=... GIT_COMMITTER_EMAIL=... \
     git branch -f public "$(git commit-tree "$tree" -m 'Initial public release')"
   git diff HEAD public | wc -l    # doit afficher 0
   ```
2. **Décider de l'identité** affichée sur les commits (nom et e-mail). Vérifiez avec `git log --format='%an <%ae>' | sort -u` ; réécrivez l'auteur du commit unique si elle ne doit pas être publique (`git commit --amend --reset-author` après avoir réglé `user.name` et `user.email`).
3. **Balayage** : `tools/public-scan.sh --files`. Les trouvailles restantes doivent toutes s'expliquer : les adresses privées sont des exemples et des données de test (plages RFC 1918), `dominet.local` est le nom par défaut du produit, les « secrets » sont des mots de passe de test et des noms de variables. Tout le reste est corrigé d'abord.
4. **Barrière** : `make ci-full` vert, `make docs` propre, `make check-migrate` vert (pile de dev arrêtée), `tools/check-installer.sh` vert.
5. **Dépendances** : `cargo audit` et `cargo deny check` sans entrée inexpliquée ; relire [Licences](licenses.md) et mettre à jour les dates « vérifié ».
6. **Version** : régler `version` dans `Cargo.toml` (espace de travail) au numéro de la version, rafraîchir `Cargo.lock` (`cargo update -w`), déplacer la section `Unreleased` de `CHANGELOG.md` sous le nouveau numéro et la date.

## 2. Créer le dépôt (privé d'abord)

Sur GitHub : un dépôt **privé** `domia-automation`, sans fichiers modèles (ils sont dans l'arbre). Poussez la branche aplatie comme `main` (la branche de travail reste locale) :

```bash
git remote add origin git@github.com:<propriétaire>/domia-automation.git
git push -u origin public:main   # le hook pre-push relance make ci sur une copie propre (environ 15 minutes)
```

Réglages à faire tant qu'il est privé : activer le **signalement privé des vulnérabilités** (Security, Code security), les modèles de tickets sont déjà dans `.github/`, ajouter des sujets (`home-automation`, `rust`, `mqtt`, `zigbee2mqtt`, `self-hosted`), une courte description, désactiver Wiki et Projects s'ils ne servent pas, protéger `main` (pas de poussée forcée).

## 3. Publier les artefacts

```bash
git tag -a v1.0.0 -m "1.0.0" && git push origin v1.0.0
PLATFORMS=linux/amd64 VERSION=1.0.0 tools/publish-image.sh   # première version : amd64 seulement ; docker login ghcr.io d'abord
make docs-deploy                              # construit le site et pousse la branche gh-pages
```

Créez la version GitHub à partir de l'étiquette avec la section de `CHANGELOG.md`. Vérifiez sur une machine propre : `git clone`, `./install.sh --dry-run`, puis une vraie installation qui télécharge l'image.

La première version est **amd64 seulement** : l'image arm64 est publiée (retirez `PLATFORMS`) après avoir été construite et démarrée au moins une fois sur une vraie machine arm64 (ADR 0063). D'ici là, les machines arm64 construisent l'image en local.

## 4. Passer en public

Seulement après un « feu vert » explicite du propriétaire : Settings, General, Danger Zone, **Change visibility**. Activez ensuite GitHub Pages depuis la branche `gh-pages`, rendez le paquet public sur GHCR et refaites l'installation sur machine propre depuis les adresses publiques.

Revenir en arrière n'est pas une vraie option : tout ce qui a été poussé est copié en quelques minutes. Si un secret fuit, **changez-le d'abord**, puis réécrivez l'historique.

## 5. Versions suivantes

`make ci-full`, mettre à jour `CHANGELOG.md`, monter la version, commiter, étiqueter, puis les artefacts :

1. **Image pour les deux architectures, construite en natif** (l'émulation est bien trop lente). Sur la machine amd64 : `make image image-smoke`, puis `docker tag dominet:dev <image>:X.Y.Z-amd64 && docker push <image>:X.Y.Z-amd64`. Sur une machine arm64 (un Raspberry Pi 5 a pris environ 67 minutes à froid) : extraire l'étiquette, `DOMINET_PULL=0 make prod-up`, puis étiqueter et pousser `:X.Y.Z-arm64`. Fusionner : `docker buildx imagetools create -t <image>:X.Y.Z -t <image>:latest <image>@sha256:<condensat amd64> <image>:X.Y.Z-arm64`, et vérifier avec `docker buildx imagetools inspect <image>:latest`. **Ne supprimez pas** les étiquettes `-amd64` et `-arm64` : ce sont les manifestes que l'index référence.
2. Si aucun source compilé n'a changé depuis la version précédente (documentation ou scripts seulement), les images existantes peuvent rester : pas de nouvelle version nécessaire.
3. `make docs-deploy` (ou construire le site et pousser `gh-pages` soi-même). Vérifiez l'auteur du commit de `gh-pages` : `mkdocs gh-deploy` utilise l'identité git locale et peut la laisser vide.
4. La release GitHub, depuis l'étiquette, avec la section de `CHANGELOG.md` comme notes.

Avant tout, un clone sur une machine propre doit encore s'installer : `git clone`, `./install.sh`. Une migration appliquée n'est jamais modifiée ; une reformulation demande une entrée comme celle de `crates/dominet-store/src/lib.rs`.
