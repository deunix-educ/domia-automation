# Releasing and going public

Maintainer's runbook. Nothing here is automatic: there is no CI service (ADR 0015), the gate is `make ci` run locally. **The repository stays private until the owner gives an explicit go.**

## 1. Before the first publication

1. **Decide the history.** The public repository can start from a single commit ("Initial public release") so the working history does not appear. Squash on a *copy* and compare trees:
   ```bash
   tree=$(git rev-parse 'HEAD^{tree}')
   GIT_AUTHOR_NAME=... GIT_AUTHOR_EMAIL=... GIT_COMMITTER_NAME=... GIT_COMMITTER_EMAIL=... \
     git branch -f public "$(git commit-tree "$tree" -m 'Initial public release')"
   git diff HEAD public | wc -l    # must print 0
   ```
2. **Decide the identity** shown on commits (name and e-mail). Check with `git log --format='%an <%ae>' | sort -u`; rewrite the author on the squashed commit if it must not be public (`git commit --amend --reset-author` after setting `user.name` and `user.email`).
3. **Scan**: `tools/public-scan.sh --files`. The remaining findings must all be explained: private addresses are examples and test data (RFC 1918 ranges), `dominet.local` is the product's default name, "secrets" are test passwords and variable names. Anything else is fixed first.
4. **Gate**: `make ci-full` green, `make docs` clean, `make check-migrate` green (stop the dev stack first), `tools/check-installer.sh` green.
5. **Dependencies**: `cargo audit` and `cargo deny check` have no unexplained entry; re-read [Licences](licenses.md) and update the "verified" dates.
6. **Version**: set `version` in `Cargo.toml` (workspace) to the release number, refresh `Cargo.lock` (`cargo update -w`), move the `Unreleased` section of `CHANGELOG.md` under the new number and date.

## 2. Create the repository (private first)

On GitHub: a **private** repository `domia-automation`, no template files (they are in the tree). Push the squashed branch as `main` (the working branch stays local):

```bash
git remote add origin git@github.com:<owner>/domia-automation.git
git push -u origin public:main   # the pre-push hook reruns make ci on a clean copy (about 15 minutes)
```

Settings to set while it is still private: enable **Private vulnerability reporting** (Security, Code security), the issue templates are already in `.github/`, add topics (`home-automation`, `rust`, `mqtt`, `zigbee2mqtt`, `self-hosted`), a short description, disable Wiki and Projects if unused, protect `main` (no force-push).

## 3. Publish the artefacts

```bash
git tag -a v1.0.0 -m "1.0.0" && git push origin v1.0.0
PLATFORMS=linux/amd64 VERSION=1.0.0 tools/publish-image.sh   # first release: amd64 only; log in first (docker login ghcr.io)
make docs-deploy                              # builds the site and pushes the gh-pages branch
```

Create the GitHub release from the tag with the `CHANGELOG.md` section. Check on a clean machine: `git clone`, `./install.sh --dry-run`, then a real installation that pulls the image.

The first release is **amd64 only**: the arm64 image is published (drop `PLATFORMS`) after it has been built and started on a real arm64 machine at least once (ADR 0063). Until then arm64 machines build the image locally.

## 4. Go public

Only after an explicit "go" from the owner: Settings, General, Danger Zone, **Change visibility**. Then enable GitHub Pages from the `gh-pages` branch, make the package public on GHCR, and re-run the clean-machine installation from the public URLs.

Going back is not a real option: assume everything pushed is copied within minutes. If a secret slips out, **rotate it first**, then rewrite the history.

## 5. Later releases

`make ci-full`, update `CHANGELOG.md`, bump the version, tag, `tools/publish-image.sh`, `make docs-deploy`. Migrations never edit an applied file; a rewording needs an entry like the one in `crates/dominet-store/src/lib.rs`.
