# Poste de développement

Ce document prépare une machine pour développer l'application. Le développement se fait de préférence sur une machine rapide (au moins 8 cœurs et 16 Go) ; la machine qui fait tourner l'application au quotidien peut être beaucoup plus modeste (voir [Mesures](measures.fr.md)).

## Installer

```bash
git clone git@github.com:deunix-educ/domia-automation.git
cd domia-automation && tools/setup-dev-host.sh --ci
```

`tools/setup-dev-host.sh` vérifie les paquets système (et affiche la commande `apt` s'il en manque), installe Rust à la version épinglée, les outils de la barrière (`cargo-deny`, `cargo-audit`, `sqlx-cli`, `cargo-leptos`), active le hook `pre-push`, démarre la pile de développement et applique les migrations ; avec `--ci`, il lance ensuite `make ci`. `tools/check-tools.sh` affiche les versions des outils attendus.

Prérequis qu'il ne peut pas installer : Docker (avec le plugin `compose` et votre utilisateur dans le groupe `docker`) et Firefox (tests de bout en bout).

## Lancer l'application en développement

```
make dev-up        # pile de dev : PostgreSQL, MediaMTX, Mosquitto, ReductStore
make dev-run       # http://127.0.0.1:8080 (DISCOVERY=1 make dev-run : avec le rôle discovery)
```

`make dev-run` (`tools/dev-run.sh`) construit le binaire et l'interface s'ils manquent, applique les migrations, crée la clé maîtresse et un compte administrateur au premier lancement, puis démarre `dominet serve`. Les accès sont gardés dans `deploy/.env.dev-run` (fichier ignoré par git, droits 600) : ne le commitez jamais et ne l'affichez pas.

## La barrière avant commit et avant push

- `make ci` : format, lints en erreurs, tests, WASM, `cargo deny`, `cargo audit`, métadonnées SQL. La pile de développement doit tourner (`make dev-up`).
- `make ci-full` : ajoute la construction de l'image Docker et les tests navigateur sur cette image ; à lancer avant un jalon ou quand le `Dockerfile` ou une dépendance native change.
- Le hook `pre-push` (`make hooks`) relance `make ci` sur une copie propre du commit poussé. Il utilise la pile de développement : ne l'arrêtez pas pendant un push. Si votre hébergeur coupe la connexion pendant ce temps : `git -c core.sshCommand="ssh -o ServerAliveInterval=30" push`.

Il n'y a pas de CI automatique : la barrière est locale (voir [Décisions par thème](decisions.fr.md)).

## Place disque

`target/` grossit vite (le hook compile une copie propre dans `target/prepush`, le profil de test dans `target/debug`) et peut dépasser plusieurs centaines de Go. `rm -rf target/prepush` libère le plus gros sans conséquence (il se reconstruit au prochain push). Ne compilez jamais sur la machine de production.
