# 0024 — E2E navigateur : rester sur Marionette/Firefox


- Statut : accepté
- Date : 2026-11-02

## Contexte
La ROADMAP prévoyait Playwright pour les E2E de R2, R6 et R8. Depuis R1 les E2E tournent avec Firefox piloté par Marionette (client maison, sans dépendance, `tests/e2e/marionette.py`),
branchés sur une base jetable, sur le binaire local et sur l'image Docker (`tools/e2e.sh`). Node, nécessaire à Playwright, est absent de l'hôte (machine de production).

## Options
(a) **Rester sur Marionette** — 11 scripts, 286 vérifications, rien à installer, déjà dans `make ci-full` ; les actions de pointeur du protocole WebDriver (`WebDriver:PerformActions`) suffisent à tester le
glisser-déposer au pointeur. (b) Installer Node et Playwright — attentes automatiques et traces, mais un second écosystème sur la machine de production et une réécriture des scripts existants.

## Décision
Option (a), jusqu'à preuve du contraire : la réévaluation se fait en R6 (live caméra WebRTC) si Marionette ne permet pas de tester la vidéo.

Règles apprises en R2, à appliquer dans tout nouveau script :
- **Attendre la fin de l'hydratation** (indicateur « Temps réel : connecté ») avant le premier clic : un bouton présent et actif avant l'hydratation n'a aucun effet.
- Utiliser un **numéro d'appareil propre au script** : le serveur de l'E2E est partagé et retient le dernier état de chaque appareil.
- Préparer des données que l'interface ne sait pas créer par `E2E_DB_SHELL` (SQL sur la base jetable) ; préférer l'interface quand elle sait le faire (`r2.py` n'utilise aucun SQL).
- Un échec doit s'expliquer : `tools/e2e.sh` affiche le début du panic du serveur ou la fin de son journal, `KEEP_SERVER_LOG` conserve le journal, `DOMINET_BIN`/`DOMINET_SITE` essaient un autre profil de compilation.

## Conséquences
- Pas de Node sur l'hôte ; les E2E restent exécutables sur la machine de production.
- Pas d'attentes automatiques : chaque attente est écrite (`wait_for`) ; un test instable est un défaut du test **ou** du produit, à élucider (deux défauts de rendu de Leptos ont été trouvés ainsi).
