# 0031 — Schéma des scènes, automations, alertes et énergie


- Statut : accepté
- Date : 2026-11-02

## Contexte
R4 reprend les scènes, les automations, les règles d'alerte et le bilan d'énergie de l'implémentation de référence. Le plan (plan de la phase, non conservé) a établi, en lisant le code, que `device_state` n'y est jamais évalué et que `mqtt_publish` n'y publie pas : on reprend donc **l'intention** des données, pas
toute la mécanique. Ce lot pose le schéma, le domaine et les dépôts ; les lots suivants y branchent le moteur.

## Décisions
- **Forme JSON de la référence en base, types forts en mémoire.** `automation.trigger_type` + `trigger_config`, `condition`, `action_type` + `action_config` gardent la forme de la référence (les scénarios de la production se chargent tels quels, sans conversion). Le domaine les lit dans des énumérations (`Trigger`, `Condition`,
  `Action`) qui valident champ par champ ; une ligne de base qui ne se relit plus est signalée comme donnée corrompue plutôt que silencieusement ignorée. les plugins sont lus comme `script` (Rhai, code dans la colonne `script`, exécuté à partir du lot 5) et réécrit `script`.
- **Scènes.** `scene` + `scene_action` (appareil, commande JSON, `position`). Le **nom est unique** (l'API de script active une scène par son nom ; la référence n'imposait rien et plantait sur un doublon). Les commandes suivent les mêmes règles que celles des widgets (objet JSON non vide, 1 Ko, 3 niveaux), au plus 100 par scène.
  La création et le remplacement des actions sont **une transaction** : jamais de scène à moitié écrite. `widget.scene_id` devient une clé étrangère (`ON DELETE SET NULL`, références orphelines mises à `NULL` avant la contrainte, comme pour les appareils en R3) : les numéros fictifs de R2 ne sont plus acceptés.
- **Journal d'exécution** `automation_run` (une ligne par déclenchement : instant, cause, statut `ok|error|skipped|limited`, durée, message borné à 1 000 caractères), supprimé avec l'automation, purgé par âge. la référence n'avait que `last_triggered` et `trigger_count` (conservés).
- **Planificateur.** `schedule_job` (migration 0006) gagne `automation_id` (unique, supprimé avec l'automation) et accepte les **neuf** événements solaires de un planificateur externe (la contrainte d'origine n'en admettait que deux).
- **Alertes.** `alert_rule` (appareil, clé, opérateur, seuil numérique ou texte, sévérité, cooldown en minutes, trois canaux) et `alert` (statut `firing|resolved|acknowledged`, valeur déclenchante, dates, acquittement par un compte). `AlertRuleDraft::evaluate` est **le calcul de la référence** : valeur et seuil lus comme nombres (texte numérique et
  booléens compris, `float(True) = 1.0`), repli sur l'égalité de textes sans casse ni espaces pour `eq` / `neq` seulement, `contains` sur le texte canonique de la valeur (`True`, `None`). Une alerte dont la règle est supprimée **reste** (sans règle) ; celles d'un appareil supprimé partent avec lui.
  `latest_firing` ne regarde que les alertes **en cours** : c'est la particularité de la référence (une alerte acquittée sort du calcul du cooldown) reproduite exprès (lot 6).
- **Énergie.** `energy_daily_log` (un jour local par ligne, quatre entiers en Wh). `recent(n)` renvoie les **n derniers** jours du plus ancien au plus récent : la référence renvoyait, par erreur, les n premiers (`order_by("date")[:30]` : les plus anciens, la page se figeait au bout de trente jours).
- **Cron.** `CronSpec::parse` reprend la syntaxe de `un planificateur externe.schedules.crontab` (listes, plages, pas, plage à l'envers qui fait le tour du champ, jours de semaine nommés, dimanche = 0) ; le calcul du prochain passage, vérifié contre un planificateur externe, est celui du lot 4.
- **Scénarios de la production.** un script de portage retiré a converti un contexte de production en `tests/fixtures/automation/prod-context.json` ; `dominet_domain::scenarios` le lit (fonctionnalité `scenarios`, jamais dans le binaire). **Aucun import** : ce sont des jeux d'essai.

## Conséquences
- Les formulaires d'administration (lots 2, 3, 5, 6) valident par les mêmes `problems()` que la base : une seule source de règles.
- Les deux plugins de la production sont chargés comme scripts « à porter » (`// a porter en Rhai`) tant que le lot 5 ne les a pas écrits.
- Le journal d'exécution grossit avec l'activité : purge à brancher avec la rétention de l'historique (lot 3).
