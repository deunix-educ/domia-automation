# 0015 — Barrière locale (hook pre-push) à la place de la CI GitHub automatique


- Statut : accepté — remplace la partie « CI automatique sur runner » de l'[ADR 0013](0013-self-hosted-ci.md)
- Date : 2026-11-01

## Contexte
L'ADR 0013 avait déplacé la CI GitHub sur un runner auto-hébergé de l'EliteDesk (gratuit, sans quota). Mais le workflow ne fait
que relancer `make ci` (+ build de l'interface et test de l'image), que le développeur lance déjà avant chaque commit sur la même
machine, qui est aussi le serveur de développement et de production. Coût réel : 8 à 10 minutes par push, un utilisateur système
dédié, Docker sans root, des plafonds de ressources à entretenir, et un échec récent (rouge) impossible à lire ou à reproduire
sans accès à l'API GitHub. Pour un projet à un seul développeur, la trace par commit n'apporte rien.

## Options
1. **Garder la CI automatique** (ADR 0013) — conforme à l'usage courant, mais lente, lourde et redondante ici.
2. **Rien d'autre que `make ci` à la main** — le plus simple, mais n'attrape pas l'oubli d'un fichier dans `git add` (tout compile
   dans l'arbre de travail, rien n'est vérifié sur ce qui sera réellement poussé).
3. **Hook `pre-push` : `make ci` sur une copie propre du commit poussé** — retenu.
4. **CI GitHub déclenchée à la demande seulement** — gardée en complément (voir Décision).

## Décision
- `.githooks/pre-push` (activé par `make hooks`, qui règle `core.hooksPath`) extrait chaque commit de tête poussé avec `git archive`
  (donc **sans fichiers non suivis ni modifications non commitées**), démarre un PostgreSQL jetable (`postgres:16-alpine`, port
  aléatoire, supprimé à la fin) et lance `make ci` en priorité CPU basse (`nice`). Échec : rien n'est poussé, les 60 dernières lignes
  et le chemin du journal sont affichés.
- Cible Cargo dédiée (`target/prepush`) : n'interfère pas avec la compilation en cours ; la première exécution est plus longue.
- `make ci-full` = `make ci` + `make front` + `make image-smoke` (build de l'interface et de l'image Docker), à lancer avant un jalon,
  quand le `Dockerfile` ou la chaîne de build change, et avant R8. `DOMINET_PREPUSH_CMD='make ci-full' git push` l'applique au push.
- Passer outre : `git push --no-verify` ou `DOMINET_SKIP_PREPUSH=1`. C'est un garde-fou, pas une barrière : un hook local ne
  protège pas contre un contournement volontaire.
- Le workflow GitHub reste dans le dépôt mais en **déclenchement manuel** (`workflow_dispatch`) ; il n'y a plus de run automatique.
  Le runner peut rester installé (inactif) ou être retiré avec `ci/runner/uninstall.sh` (droits root).

## Conséquences
- Un push prend le temps de `make ci` sur une copie propre (quelques minutes, moins à chaud) au lieu de 8 à 10 minutes distantes.
- Les tests tournent sur la machine de production, mais uniquement contre un PostgreSQL jetable : jamais le Mosquitto, les caméras ni le
  dongle Zigbee de la prod. Ils consomment seulement du CPU (`nice`).
- Plus de statut CI sur GitHub : la preuve que le commit a passé les contrôles est le fait d'avoir pu pousser. Si le projet gagne des
  contributeurs, revenir à une CI automatique (ADR 0013, ou machines GitHub) redevient pertinent.
- Les risques propres au runner (ADR 0013) disparaissent tant qu'il n'est pas utilisé.
