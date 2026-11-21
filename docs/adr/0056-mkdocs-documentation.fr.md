# ADR 0056 — Documentation : MkDocs dans l'image, servie sur `/docs/` derrière la connexion


Statut : accepté (R7 lot 7, décision D10 du plan).

## Contexte
L'implémentation de référence publie une documentation MkDocs Material FR/EN sur `/docs/`, derrière la connexion. Le dépôt Rust a déjà ses pages (`docs/*.md` et `*.en.md`, ADR, synthèses) mais aucun site : elles ne se lisent que dans le dépôt. L'application doit servir sa propre documentation, sur la machine installée, sans accès extérieur.

## Options
1. **Construire au `docker build`** (étape de documentation jetée) et servir le site statique par Axum (retenu, D10).
2. Construire en local et copier le site dans l'image : image plus légère, mais un build de plus à penser et un site qui peut ne pas correspondre au code.
3. Servir le Markdown et le rendre dans l'application : un moteur de plus à maintenir, sans recherche ni navigation.

## Décision
- `mkdocs.yml` à la racine : thème Material, `mkdocs-static-i18n` en mode **suffixe** (`page.md` = français, `page.en.md` = anglais ; une page sans version anglaise retombe sur le français, c'est le cas des ADR). Versions figées dans `requirements-docs.txt`. Le site est écrit dans `target/docs-site`.
- La navigation est écrite à la main (fonctions, exploitation, développement, ADR, synthèses). Les plans de phase `R*-plan.md`, le gabarit d'ADR et `mesures-r6/` sont exclus ; les ADR restent atteignables par l'index. `DEPLOY.md` (racine) est relié par un lien symbolique `docs/deployment.md` et un hook (`docs/hooks/links.py`) qui corrige ses liens `docs/…`.
- `make docs` construit en mode **strict** (un lien cassé fait échouer) dans un venv `~/.venvs/dominet-docs` ; `make docs-serve` donne un aperçu. Le `Dockerfile` a une étape `docs` (`python:3.13-slim-trixie`, `mkdocs build --strict`) dont seul le site est copié dans `/app/docs` ; `DOCS_ROOT` désigne le dossier (`target/docs-site` par défaut).
- `dominet-server::web::docs` : `ServeDir` monté sur `/docs`, derrière un middleware qui exige une session valide (le même extracteur que les autres pages). Un visiteur anonyme est renvoyé vers `/login?next=…` sans qu'une ligne de documentation ne parte. `Cache-Control: private, max-age=300`. Aucun rôle particulier n'est exigé : tout compte connecté lit la documentation.

## Conséquences
- L'image grossit d'environ 12 Mo ; le build de l'image demande Internet pour `pip` (comme pour `cargo`).
- Le pare-feu de la machine n'a rien à ouvrir de plus : `/docs/` passe par nginx comme le reste de l'application.
- Material pour MkDocs annonce sa fin de vie au 5 mai 2027 (successeur : Zensical, compatible avec `mkdocs.yml`) ; la migration sera à étudier avant cette date. Versions figées d'ici là.
