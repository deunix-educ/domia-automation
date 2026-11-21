# 0006 — Scripts d'automation : hôte Rhai sandboxé (spike 7)


- Statut : accepté
- Date : 2026-10-31
- Verdict du spike : **GO**

## Contexte
Dans la version précédente, les plugins et le code de traitement des sources exécutaient du code utilisateur dans un autre langage. En Rust, les
scripts passent par **Rhai** embarqué. Exigences (ROADMAP R4) : API `publish/get_device/scene/log_value`,
variables `topic`/`payload`/`device`, limites (opérations, profondeur, taille, durée) et un script
hostile interrompu sans impact sur le processus. Code du spike : `spike (code non conservé)` (Rhai 1.27.0,
features `sync`, `serde`, `no_module`).

## Résultats
- 18 vérifications, 0 échec : script légitime (API complète), `get_device` inconnu → `()`, topic invalide refusé.
- Scripts hostiles tous interrompus en < 45 ms, mémoire stable (RSS max 5,6 → 6,7 Mo) : boucles infinies, récursion infinie, croissance de chaîne/tableau/map, `pad` à 2·10⁹, `eval`, `import`, imbrication de 5000 niveaux (rejetée au parsing), 100+ `publish`.
- Limite de durée (`on_progress`) validée seule, opérations illimitées : arrêt à 250 ms.
- Panique d'une fonction hôte : isolée par un thread, le processus survit.
- Performance : script compilé une fois (AST), ≈ 1 µs/exécution, ≈ 850 000 exécutions/s (sans le coût de conversion JSON réaliste, de l'ordre de la dizaine de µs).

## Décision
- Hôte Rhai dans `dominet-automation`, **moteur reconstruit par exécution** (peu coûteux) avec : `Engine::new_raw` + `StandardPackage`, `set_max_operations(200_000)`, `set_max_call_levels(24)`, `set_max_expr_depths(48, 24)`, chaînes ≤ 16 Ko, tableaux ≤ 2 000, maps ≤ 500, `disable_symbol("eval")`, `no_module` (pas d'`import`), délai mural de 250 ms via `on_progress`. Les valeurs sont des paramètres de réglage (`AppSetting`), pas des constantes figées.
- **Aucun effet de bord direct** : `publish`, `scene`, `log_value` ajoutent des `Effect` (max 100 par exécution) que le moteur applique après coup ; `get_device` lit un instantané fourni par l'hôte. Les topics `publish` sont validés (pas de joker, ≤ 256 caractères).
- `print` est redirigé vers le journal d'exécution des automations ; `debug` est ignoré.
- Exécution dans `spawn_blocking` (ou un thread dédié) : une panique d'une fonction hôte se traduit par une erreur d'exécution journalisée, pas par l'arrêt du processus. Les ASTs sont mis en cache par automation (invalidés à la modification).

## Conséquences
- La limite de durée n'agit qu'entre deux opérations Rhai : une fonction native lourde ne serait pas interrompue. Les tailles bornées (tableaux, chaînes) et l'absence de fonctions hôtes bloquantes l'évitent ; toute future fonction hôte devra être non bloquante et bornée.
- Les scripts existants de la référence ne se portent pas tels quels : à réécrire en Rhai à partir de `context.yaml` (cohérent avec le démarrage à vide).
- Éditeur UI (CodeMirror) et journalisation d'exécution restent à livrer en R4.
