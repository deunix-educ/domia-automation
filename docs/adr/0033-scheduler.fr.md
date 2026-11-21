# 0033 — Planificateur (cron, soleil) et calcul solaire


- Statut : accepté
- Date : 2026-11-02

## Contexte
L'implémentation de référence s'appuyait sur un planificateur externe (un planificateur externe) : une `CrontabSchedule` (fuseau `Europe/Paris`) ou une `SolarSchedule` par automation `schedule` / `solar`, et `ephem` pour le soleil. Le plan R4 (D6, D7) demande de s'en passer, avec un calcul **vérifié contre l'implémentation de référence**.

## Décisions
- **Un rôle `scheduler`**, dans le processus `serve` par défaut (avec `web`), seul exemplaire d'une installation. Il lit la table `schedule_job` (migration 0006) ; le prochain passage est persisté (`next_run_at`) pour l'administration mais **recalculé** au démarrage et à chaque changement de définition. Il dort jusqu'au prochain passage, au plus 30 s, et se réveille quand l'administration modifie une automation (`schedule_changed`).
- **Pas de rattrapage** (comme un planificateur externe). Un passage découvert plus de 60 s après son heure (serveur arrêté, base lente) n'est pas rejoué : le travail note « manqué (retard de N s, pas de rattrapage) » et passe au suivant.
- **Synchronisation avec les automations.** Chaque automation active à déclencheur `schedule` ou `solar` a un travail `automation-<id>` (tâche `run_automation`) qui suit sa définition ; désactivée, supprimée ou devenue `mqtt_message`, son travail disparaît. Une définition inchangée garde son prochain passage. Le travail système `compute_daily_energy` (`5 0 * * *`, fuseau `TZ`) est créé s'il manque et n'est jamais écrasé ensuite (on peut le modifier) ; son calcul arrive au lot 7.
- **Cron.** Syntaxe de un planificateur externe (ADR 0031), heure murale du fuseau du travail (`TZ`, `Europe/Paris` par défaut). Le jour du mois **et** le jour de la semaine doivent correspondre (un planificateur externe, pas Vixie cron). **Changements d'heure comme `TzAwareCrontab`** : une heure inexistante (saut de printemps, 02:30) se lit avec le décalage d'avant le saut (donc 03:30 d'été), une heure répétée (retour d'automne) est prise à sa première occurrence et sa seconde occurrence ne déclenche rien. Chaque comportement est dans le fichier d'oracle.
- **Soleil sans dépendance.** `dominet_domain::solar` : séries de Meeus pour la position du soleil, équation du temps, angle horaire. Mêmes conventions que `un planificateur externe.schedules.solar` : lever et coucher au bord supérieur à −34′ (centre ≈ −0,83°), aubes et crépuscules au centre à −6°, −12°, −18°, midi solaire = passage au méridien. Soleil de minuit ou nuit polaire : pas de passage dans les trois jours qui suivent, donc pas d'événement ; le travail réessaie 6 h plus tard (un planificateur externe lève une erreur dans ce cas).
- **Intervalle.** Supporté par la table et par le calcul (`après + n × unité`) ; aucune automation n'en produit aujourd'hui.
- **Exécution.** Le planificateur déclenche l'automation par le **même moteur** que les messages (`Engine::fire` : garde-fou, journal, compteurs), donc scène, publication et notification se comportent à l'identique. Un processus `--only scheduler` ouvre son propre client MQTT (sans écoute) si un broker est configuré.

## Vérification contre la référence
- 119 suites de six passages d'un cron (17 expressions × 7 points de départ : nuits de changement d'heure, fin d'année, 29 février) contre `TzAwareCrontab` : **identiques à la seconde**.
- 1 036 événements solaires (9 événements × 5 lieux × 12 mois × 2 heures de départ, dont Reykjavik et Sydney) contre `un planificateur externe.schedules.solar` : **écart maximal 13 s** (tolérance du plan : 90 s). Les cas où un planificateur externe lui-même échoue (soleil de minuit) sont écartés.

## Conséquences
- Plus de un planificateur externe, de Redis ni de un planificateur externe : un seul processus, une seule table.
- La précision du soleil dépend de séries à ~0,01° : largement suffisante pour des volets, à revoir seulement au-delà des cercles polaires.
- `chrono-tz` (base IANA embarquée) entre dans le binaire du serveur, jamais dans le WASM (le domaine n'en dépend pas).
