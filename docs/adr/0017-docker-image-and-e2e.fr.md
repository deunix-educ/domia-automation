# 0017 — Image Docker avec `cargo-leptos` et E2E sur l'image


- Statut : accepté
- Date : 2026-11-01

## Contexte
Jusqu'au lot 8 l'image ne contenait que le binaire serveur (pas de fichiers WASM/JS/CSS : les pages se rendaient mais ne s'hydrataient pas). Le lot 9
livre l'image complète. Un défaut rencontré à la fin du lot 8 impose aussi une exigence : le serveur (HTML rendu) et le WASM (hydratation) doivent
être compilés **de la même façon**, sinon l'interface s'affiche mais reste inerte (boutons désactivés, aucun message) ; ni `clippy` ni les tests
Rust ne le voient.

## Options
1. **`cargo leptos build --release` dans l'image**, avec `cargo-chef` pour le cache des dépendances (serveur et WASM) — retenu.
2. Compiler le WASM hors de l'image (poste de développement) et le copier — non reproductible, dépend de la machine.
3. Hachage des noms de fichiers (`hash-files`) pour un cache long — écarté pour l'instant : le fichier d'empreintes doit se trouver à côté du
   binaire, ce qui ajoute une dépendance de plus entre binaire de dev, `cargo leptos` et image (la classe de défaut qu'on vient de corriger) ;
   `/pkg/*` reste en `no-cache` (revalidation, réponse 304 de quelques octets).
4. **arm64** — voir Décision.

## Décision
- **`Dockerfile`** : `cargo-leptos` 0.3.11 (version épinglée) et `cargo-chef` installés dans une couche de base ; deux `cargo chef cook` (serveur, et WASM
  avec la même cible, le même profil `wasm-release` et le même dossier `target/front` que `cargo leptos build --release`) ; `cargo leptos build --release`
  produit le binaire et `target/site` ; l'image finale (Debian slim, utilisateur 10001) contient `/usr/local/bin/dominet` et `/app/site` (`SITE_ROOT`).
  `.cargo/config.toml` est copié dès la première étape : il porte les réglages communs serveur/WASM. `cargo-leptos` télécharge `wasm-bindgen` (version de
  `Cargo.lock`) et `wasm-opt` pendant le build : il faut le réseau. Résultat : WASM de **1,3 Mo** (20 Mo en dev), image de 137 Mo.
- **Test de fumée** (`ci/image-smoke.sh`) : santé, `/readyz` sans base, non-root, arrêt propre, **et** `/login` (200, CSP à nonce, fichier WASM attendu,
  marqueurs d'hydratation), `/pkg/*` (200, `application/wasm`, gzip, `no-cache`, taille plausible).
- **E2E sur l'image** (`tools/e2e.sh --image`, `make e2e-image`) : conteneur sur le réseau de l'hôte, base jetable créée puis supprimée, Firefox
  headless à profil temporaire, les 7 scripts de `tests/e2e/`. C'est le seul test qui hydrate la page avec le binaire et le WASM **de production** ;
  vérifié en retournant un échec (code 1, « Interface interactive : 0 clic ») quand les fichiers statiques sont absents. Même lanceur sans `--image`
  pour le binaire de développement (`make e2e`).
- `make ci-full` = `make ci` + `image-smoke` + `e2e-image`, à lancer avant un jalon et quand le `Dockerfile` ou la chaîne de build change
  (`DOMINET_PREPUSH_CMD='make ci-full' git push` l'applique au push). `make ci` reste la barrière ordinaire (ADR 0015).
- **arm64 non construit.** Cette machine n'a pas d'émulation (`exec format error`) et configurer `binfmt`/QEMU demande les droits root ; la ROADMAP en fait
  un point non bloquant tant que le Raspberry Pi 5 n'est pas retenu. À reprendre avec un runner arm64 natif : le `Dockerfile` n'a aucune dépendance à
  l'architecture, seules les versions de `wasm-bindgen`/`wasm-opt` téléchargées devront exister pour `aarch64`.
- Pas de `HEALTHCHECK` dans l'image (l'image slim n'a pas `curl`) : la sonde `/healthz` sera déclarée dans le `docker-compose` de production (R7).

## Conséquences
- Un changement du `Dockerfile`, de `.cargo/config.toml`, de `Cargo.toml` (profils, métadonnées Leptos) ou des versions d'outils se valide avec `make ci-full`.
- Premier build à froid long (cargo-leptos, dépendances du serveur et du WASM) ; ensuite seul le code du projet est recompilé : un changement d'une ligne rebâtit l'image en ≈ 2 min 30 (mesuré).
- Les outils téléchargés pendant le build (`wasm-bindgen`, `wasm-opt`) sont une dépendance réseau du build ; une image de base interne ou un cache de
  ces binaires pourra les remplacer.
