# 0013 — CI sur un runner auto-hébergé (EliteDesk)


- Statut : accepté, puis **partiellement remplacé par l'[ADR 0015](0015-local-gate.md)** (la CI n'est plus automatique : barrière locale `pre-push`). Le runner et son installation restent documentés ici.
- Date : 2026-11-01

## Contexte
La CI sur les machines de GitHub était lente (runs à froid de plusieurs dizaines de minutes, caches à restaurer/exporter,
image Docker reconstruite sans couches) et soumise à un quota de minutes (≈ 2 000 par mois en dépôt privé, plan gratuit).
Le développeur veut une CI rapide, sans quota. Les mêmes contrôles tournent déjà en local (`make ci`).

## Options
1. **Machines GitHub** (statu quo) — aucune maintenance, mais lent, plafonné, et il faut gérer des caches distants.
2. **Runner auto-hébergé sur l'EliteDesk, sans isolation** (Docker de la machine, compte du développeur) — rapide, mais un
   workflow ou une action tierce compromis aurait un accès équivalent à root (groupe `docker`) sur le serveur de production.
3. **Runner auto-hébergé sur l'EliteDesk, isolé** (utilisateur dédié sans sudo, Docker sans root, ressources plafonnées) — retenu.
4. **Autre serveur dédié** — meilleure séparation avec la prod ; reste possible plus tard (même script d'installation).
5. **Pas de CI distante, `make ci` seul** — rapide et gratuit, mais aucun contrôle sur machine propre ni trace par commit.

## Décision
- Option 3, **uniquement ce dépôt privé**. `ci/runner/install.sh` installe tout de façon idempotente (avec simulation `DRY_RUN=1`) :
  utilisateur `gha-runner` verrouillé et hors des groupes `sudo`/`docker`, Docker sans root pour lui seul, Rust et outils installés une
  fois, runner vérifié par empreinte SHA-256, service systemd utilisateur à redémarrage automatique, nettoyage hebdomadaire du cache Docker.
- **Plafonds de ressources** sur tout ce que lance le runner (tranche `user-<uid>.slice`) : poids CPU 20 (cède la place aux autres
  services en cas de charge), mémoire 4 Go « haute » et 5 Go maximum.
- **Le workflow lance `make ci`** : une seule définition de la CI, identique en local et sur le runner ; plus aucune action tierce
  (seul `actions/checkout`, officiel). La base de test est un conteneur PostgreSQL jetable par job (port 55432).
- Caches **sur le runner** : cible Cargo persistante (`~/ci-target`, hors de l'espace de travail que `checkout` nettoie), couches Docker
  du Docker sans root, outils préinstallés (`ci/runner/check-tools.sh` échoue clairement s'il en manque un).
- Garde-fou : les pull requests venant d'un **fork** ne s'exécutent pas sur ce runner.
- Pas de repli sur les machines GitHub (choix de l'utilisateur). L'arrêt du runner fait attendre les jobs ; `make ci` en local reste
  la barrière.
- Le job arm64 en émulation est **supprimé** du workflow (lent et gourmand) ; à reprendre avec un runner arm64 natif (le Pi 5, s'il est retenu).

## Conséquences
- La CI devient rapide (caches chauds : quelques minutes) et ne consomme plus de minutes GitHub.
- Elle partage la machine avec le reste : le runner est bridé, mais une compilation complète reste un pic de charge. À surveiller quand
  la prod DomiNet tournera sur la même machine ; déplacer le runner sur un autre serveur est alors le bon réflexe.
- Le runner exécute du code du dépôt : ne jamais l'utiliser pour un dépôt public ni pour des PR de fork, ne pas y stocker de secrets.
- Maintenance : mise à jour du runner (version fixée dans le script), surveillance de l'espace disque (`~/ci-target`, Docker), outils de
  la CI à réinstaller quand leurs versions changent (`check-tools.sh` signale l'écart).
