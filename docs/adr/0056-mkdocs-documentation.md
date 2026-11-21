# ADR 0056 — Documentation: MkDocs built into the image, served on `/docs/` behind the login

Status: accepted.

## Context
The application publishes MkDocs Material documentation in two languages on `/docs/`, behind the login. The pages exist in the repository (`docs/*.md`, ADRs) but without a site they can only be read in the repository. The application must serve its own documentation, on the installed machine, without outside access.

## Options
1. **Build at `docker build`** (a throw-away stage) and serve the static site through Axum (chosen).
2. Build locally and copy the site into the image: a lighter image, but one more build to think about and a site that may not match the code.
3. Serve the Markdown and render it in the application: one more engine to maintain, without search or navigation.

## Decision
- `mkdocs.yml` at the root: Material theme, `mkdocs-static-i18n` in **suffix** mode (`page.md` = English, `page.fr.md` = French; a page without a French version falls back to English). Versions pinned in `requirements-docs.txt`. The site is written to `target/docs-site`.
- Navigation is written by hand (features, operations, development, ADRs). ADR templates and bench data are excluded; ADRs stay reachable through the index. `DEPLOY.md` (root) is linked by a symbolic link `docs/deployment.md` and a hook (`docs/hooks/links.py`) that fixes its `docs/…` links.
- `make docs` builds in **strict** mode (a broken link fails) in a venv `~/.venvs/dominet-docs`; `make docs-serve` gives a preview. The `Dockerfile` has a `docs` stage (`python:3.13-slim-trixie`, `mkdocs build --strict`) of which only the site is copied to `/app/docs`; `DOCS_ROOT` designates the folder (`target/docs-site` by default).
- `dominet-server::web::docs`: a `ServeDir` mounted on `/docs`, behind a middleware that requires a valid session (the same extractor as the other pages). An anonymous visitor is sent to `/login?next=…` without a line of documentation leaving. `Cache-Control: private, max-age=300`. No particular role is required: any signed-in account reads the documentation.

## Consequences
- The image grows by about 12 MB; building the image needs the Internet for `pip` (as for `cargo`).
- The machine's firewall has nothing more to open: `/docs/` goes through nginx like the rest of the application.
- Material for MkDocs announces its end of life on 5 May 2027 (successor: Zensical, compatible with `mkdocs.yml`); migration should be studied before that date. Versions pinned until then.
