# ADR 0058 — Serveur MCP intégré au binaire : `rmcp`, HTTP sans session, crate `dominet-mcp`


Statut : accepté, validé par l'utilisateur le 2026-11-16 (R11 lot M0 ; décisions de l'utilisateur du 2026-11-13 : serveur intégré sur `/mcp`, LAN et VPN seulement, scripts Rhai modifiables, messagerie exclue, tout appareil actif pilotable).

## Contexte
Un assistant (Claude Code, Claude Desktop, ou l'assistant local de R12) doit pouvoir consulter, piloter et configurer DomiNet avec les mêmes règles que l'interface web, sans jamais voir un secret. L'audit du lot M0 ([R11-audit](../mcp-audit.md)) montre que les 107 fonctions serveur de l'interface suivent toutes le même schéma (`AppState`, contrôle d'accès, logique sur les dépôts) : la logique est extractible, rien n'est à inventer côté API.

## Options
1. **Binaire séparé en `stdio`** qui rappelle les `/sf/...` : duplique les schémas, dépend du format interne des fonctions serveur, à installer sur chaque poste. Écarté le 2026-11-13.
2. **Service séparé (la référence, Node)** qui appelle `/mcp` avec un jeton : un déploiement, un secret et une surface réseau de plus.
3. **Serveur intégré au binaire `dominet`, transport Streamable HTTP** (retenu).

Pour le montage : (a) `dominet-server` dépend de `dominet-mcp` et lui passe un trait de services ; (b) `dominet-mcp` dépend de `dominet-server` et **le binaire** monte ses routes. Pour le SDK : `rmcp` (officiel) ou une implémentation à la main du JSON-RPC.

## Décision
- **Crate `dominet-mcp`**, qui dépend de `dominet-server` (`AppState`) ; le binaire `dominet` passe `dominet_mcp::routes` à `server::serve`, qui les ajoute au routeur **sous les mêmes middlewares** (journal, identifiant de requête, délai de 30 s, en-têtes de sécurité). Pas de dépendance circulaire, pas de trait de 35 méthodes à maintenir en parallèle des services. Un client hors navigateur (sans `Origin`) n'est pas concerné par le contrôle CSRF.
- **SDK `rmcp` 3.10.0**, version **épinglée** (`=3.10.0`) : le protocole évolue vite, une montée de version est une tâche explicite. Licence Apache-2.0, acceptée par `cargo deny` ; aucun avertissement `cargo audit` nouveau.
- **Sans session** : `legacy_session_mode = false` et réponses JSON simples (`json_response`). Chaque requête est autonome, ce qui convient à des outils qui répondent en quelques millisecondes et laisse le serveur sans état par client. Un client de la révision précédente du protocole fonctionne aussi sans session (testé).
- **Protection contre le DNS rebinding** : l'en-tête `Host` doit être `localhost`, `127.0.0.1`, `::1` ou figurer dans `ALLOWED_HOSTS` ; sinon 403.
- **Désactivé par défaut** : `MCP_ENABLED=false` ; activé sans jeton valide, le démarrage est refusé.
- **Authentification** `Authorization: Bearer` contrôlée par un middleware avant le SDK ; ni l'en-tête ni le jeton ne sont journalisés (voir l'[ADR 0059](0059-mcp-access-tokens.md)). Pour le lot M0 seulement, un jeton provisoire `MCP_SPIKE_TOKEN` en tient lieu.
- **Lecture de la configuration en portée `lecture`** (validé le 2026-11-16) : `config_list`, `config_get` et `config_schema` n'exigent pas `configuration` ; les secrets restent masqués. Un jeton de diagnostic peut tout lire sans rien modifier.
- **Un seul chemin d'écriture** (M4) : l'interface et MCP enregistrent par `dominet_server::config_service`, qui porte les contrôles au-delà du brouillon du domaine (script qui compile, scène existante, conflits de nom) et le rechargement des parties vivantes. Le `dry_run` MCP appelle les mêmes contrôles : il annonce les refus d'une écriture réelle.
- **Outils par domaine** (environ 35, pas 110) ; chaque outil appelle la **même fonction de service** que la page web. Aucune règle métier dans `dominet-mcp`.
- **Le script Rhai est une portée à part** : tout `config_upsert` dont la charge contient du code Rhai (action `script` d'une automation, code de traitement d'une source MQTT) exige la portée `administration`, pas `configuration` (constat 1 de l'audit).

## Conséquences
- En mode sans session, `rmcp` appelle la fabrique du service **à chaque requête** (parfois deux fois : recherche du schéma de l'outil, puis traitement) : la table d'outils est donc reconstruite à chaque appel. Négligeable pour un outil, à mesurer quand le catalogue approchera de 35.
- Il faudra **extraire la logique des fonctions serveur** dans `dominet-server` (lots M2 à M5) sous la forme `service(&AppState, &Acteur, entrée)` ; l'interface en profite.
- La spécification MCP et `rmcp` évoluent : version épinglée, crate isolée, instantané des schémas d'outils dans les tests.
- Le délai de 30 s du routeur borne aussi les outils ; si un outil doit durer plus longtemps, il aura sa propre route.
- Non traité ici : OAuth 2.1 (inutile sur LAN et VPN), mode `dominet mcp --stdio` (possible plus tard sur la même crate).
