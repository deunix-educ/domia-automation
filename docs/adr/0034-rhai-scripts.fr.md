# 0034 — Scripts Rhai : mise en œuvre (automations et code de traitement des sources)


- Statut : accepté
- Date : 2026-11-02

## Contexte
L'ADR 0006 a retenu Rhai sandboxé (spike GO). Ce lot l'intègre au moteur d'automations (ADR 0032) et l'ouvre au `handler_code` des sources MQTT, avec un éditeur et un essai à blanc (plan R4, D4, D5, D11).

## Décisions
- **Hôte dans `dominet-automation::script`**, comme l'ADR 0006 : moteur reconstruit à chaque exécution, `new_raw` + paquet standard, 200 000 opérations, 250 ms, profondeur 24 / 48, chaînes 16 Ko, tableaux 2 000, tables 500, pas d'`eval` ni d'`import`, exécution dans `spawn_blocking`. **Écarts avec l'ADR 0006** : les limites sont des constantes (`Limits::default()`, passées en paramètre) et non des réglages `AppSetting` — rien ne les justifie encore en administration ; pas de cache d'AST (compilation de l'ordre de la dizaine de microsecondes, négligeable devant la base).
- **Tout ou rien.** Les effets (`publish`, `scene`, `log_value`) sont collectés puis appliqués dans l'ordre **seulement si le script se termine sans erreur** : un script qui publie puis plante ne laisse pas un état à moitié appliqué (la référence appliquait au fil de l'eau). Un effet refusé à l'application (broker coupé, scène inconnue) n'arrête pas les suivants et rend l'exécution en échec au journal.
- **Clé absente = `()`.** L'ADR 0006 faisait échouer l'accès à une clé inexistante ; avec `payload.moving == "UP"` sur un état qui n'a pas `moving` (cas courant : les états Zigbee2MQTT sont partiels), c'était une erreur à chaque appui. L'accès rend `()` (comme `payload.get("clé")` dans la référence) ; une faute de frappe devient silencieuse, `print` aide à la trouver.
- **Le port `Executor` s'étend** : `publish` prend un texte, `scene_by_name`, `log_value` et `devices` (instantané des appareils actifs, lu seulement quand une action est un script) s'ajoutent. Les scripts passent donc par le même moteur, le même garde-fou et le même journal que les autres actions.
- **Contexte de déclenchement** : l'événement porte le topic (celui de l'appareil pour un état), le message et l'appareil concerné ; un déclenchement horaire ou solaire donne un contexte vide.
- **Code de traitement des sources.** L'ingestion annonce (`Sink::source_handler`) les messages des sources actives qui ont du code ; le moteur les exécute dans la même file. Pas d'automation donc pas de ligne `automation_run` : résultat au journal du serveur. **Sourdine de 10 s** d'une source dont le code échoue : une source bavarde au code défectueux (ou hostile) ne monopolise pas le moteur (au pire 250 ms toutes les 10 s). Un code correct n'est pas limité en nombre (une source comme Linky envoie un message par seconde).
- **Autre syntaxe refusée avec un message clair.** À l'enregistrement, `script::check` reconnaît une autre syntaxe (`def`, `import`, `from … import`, `class`, `elif`, `context.`, `logger.`, deux-points en fin de ligne) avant d'essayer de compiler, puis rend l'erreur de l'analyseur Rhai (ligne, colonne). Le champ affiche `Script invalide : …` (`ProblemKind::Syntax`).
- **Éditeur (D5)** : zone de code en police fixe (`FieldKind::Code` du CRUD générique) et **essai à blanc** (`scripts/dry-run`, administrateurs) : même hôte, vrais états d'appareils, effets décrits et non appliqués. CodeMirror reste possible plus tard sans changer l'API.
- **Plugins de la production (D4).** Le implémentation de référence n'étant pas fourni, `Volet-SB-est ← gauche (open/stop)` et `→ droite (close/stop)` sont écrits d'après leur nom (`crates/dominet-domain/data/volet-sb-est-*.rhai`) : l'appui arrête un volet en mouvement, sinon ouvre (gauche) ou ferme (droite). **À relire par l'utilisateur** avant la bascule.

## Conséquences
- La limite de durée n'agit qu'entre deux opérations Rhai : toute fonction hôte future devra être non bloquante et bornée (aujourd'hui toutes sont des ajouts en mémoire).
- Un script lent (250 ms) retarde les événements suivants de la file unique ; la file bornée écarte et compte ce qui déborde.
- Les scripts hostiles de l'ADR 0006 sont tous rejoués en tests unitaires, en essai à blanc et en E2E (le serveur répond pendant et après).
