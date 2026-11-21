# ADR 0059 — Jetons d'accès MCP : hachés, à portées, révocables, avec journal d'audit


Statut : accepté, validé par l'utilisateur le 2026-11-16 : option 3, jetons personnels créés dans l'administration (R11 lot M0 ; mise en œuvre au lot M1).

## Contexte
L'interface s'authentifie par cookie de session, inutilisable par un client MCP. Un assistant qui modifie la configuration doit s'identifier, avoir des droits limités et laisser une trace. Le service est joignable par le LAN et le VPN seulement.

## Options
1. **Un jeton unique en variable d'environnement** : simple, mais sans portée, sans révocation individuelle, sans trace par jeton. Retenu **uniquement** comme jeton provisoire du lot M0 (`MCP_SPIKE_TOKEN`).
2. **OAuth 2.1** (prévu par la spécification MCP pour les serveurs publics) : lourd, inutile sur un réseau privé. Écarté.
3. **Jetons personnels créés dans l'administration** (retenu).

## Décision
- **Création** par un administrateur, dans la page d'administration ; la valeur est **affichée une seule fois**. Format : 32 octets aléatoires, préfixe `dnm_` (repérable par un détecteur de secrets), encodage URL-safe.
- **Stockage haché** (SHA-256 de la valeur : le jeton est aléatoire à forte entropie, un hachage lent n'apporte rien) avec : nom, portées, date de création, date d'expiration obligatoire (90 jours par défaut, 365 au plus), dernier usage, état (actif / révoqué). Jamais la valeur en clair, ni en base, ni dans les journaux, ni dans les sauvegardes.
- **Portées** cumulables : `lecture`, `pilotage`, `configuration`, `administration`. Un outil déclare sa portée ; l'appel est refusé sans elle. Le conseil d'usage est un jeton `lecture` pour le diagnostic courant.
- **Révocation** immédiate (un jeton révoqué ou expiré est refusé à la requête suivante).
- **Limitation de débit** par jeton.
- **Journal d'audit** : chaque appel qui **modifie** quelque chose est enregistré (jeton, outil, paramètres sans secret, résultat, instant), consultable dans l'administration. L'assistant intégré de R12 y inscrit ses actions avec la marque `agent` et le compte de la session.
- **Acteur** : le service commun reçoit un acteur (`Jeton{portées}` ou `Session{compte}`), pas seulement un compte, afin que l'assistant intégré (sans jeton) et l'interface partagent le même code (constat 3 de l'audit).
- **Hors périmètre, définitivement** : la création de jetons, les comptes et mots de passe, les comptes MQTT, la messagerie, les sauvegardes, tout accès au système de fichiers ou au shell.

## Conséquences
- Une table de jetons et une table d'audit (migrations du lot M1), deux pages d'administration, un extracteur `Bearer` qui remplace le jeton provisoire.
- Un jeton perdu se révoque et se recrée ; il n'existe aucun moyen de le relire.
- Le journal d'audit grossit avec l'usage : purge à prévoir (durée de conservation à fixer en M1).

## Mise en œuvre (lot M1, 2026-11-16)
- **Format** : `dnm_` + 43 caractères (32 octets d'aléa en base64 URL-safe sans remplissage, source `aes_gcm::Key::generate`). Le début visible est `dnm_` + 8 caractères (48 bits sur 256).
- **Table `mcp_token`** : empreinte SHA-256 (32 octets, contrainte de longueur et d'unicité), portées en `TEXT[]` (au moins une), `expires_at` **obligatoire**, `last_used_at` (écrit au plus une fois par minute), `revoked_at`. Aucune colonne ne peut contenir la valeur ; un test le vérifie sur le contenu brut des lignes.
- **Authentification** à chaque requête, depuis la base : un jeton révoqué ou expiré est refusé à l'appel suivant, sans cache. Le jeton provisoire `MCP_SPIKE_TOKEN` du lot M0 est **supprimé**.
- **Portées appliquées en un seul endroit** (`call_tool` et `list_tools` de la crate `dominet-mcp`) : un outil absent de la table `SCOPES` est refusé à tous, et un test impose que chaque outil y figure. Un refus renvoie un résultat d'erreur explicite (l'assistant peut l'expliquer) et écrit une ligne `denied` dans l'audit.
- **Limite de débit** : fenêtre glissante d'une minute par jeton, en mémoire (`MCP_RATE_LIMIT_PER_MIN`, 120) ; au-delà, 429 avec `Retry-After`. Un redémarrage remet les compteurs à zéro. Les appels refusés ne sont pas comptés.
- **Audit** (`mcp_audit`) : `token_created`, `token_revoked`, `denied`, et `tool_call` dès le lot M3. Les paramètres passent par `redact` (clés `password`, `secret`, `token`, `key`, `credential`, `authorization`… masquées à toute profondeur, textes longs coupés à 500 caractères). Purge quotidienne au-delà de `MCP_AUDIT_RETENTION_DAYS` (365).
- **Écart assumé en M1, levé en M3** : pour la création et la révocation de jetons (faites par un administrateur connecté), l'écriture de l'audit est « au mieux ». **Pour les outils qui modifient quelque chose (lot M3), l'audit est écrit avant l'exécution** (ligne `pending`, refermée en `ok` ou `error`) **et l'appel est refusé si l'écriture échoue** ; un test supprime la table d'audit et vérifie que rien n'est publié.
- **Propriétaire d'un jeton** : `Actor::Token` porte le compte qui l'a créé. L'acquittement d'une alerte lui est attribué ; un jeton dont le créateur a été supprimé ne peut plus acquitter.
- **Acteur** : `Actor::Token{id, nom, portées}` ou `Actor::Session{compte, rôle}` dans `dominet-domain` ; une session d'administrateur détient les quatre portées, tout autre compte `lecture` et `pilotage`. Seul le premier est utilisé en M1 ; le second servira à l'assistant intégré de R12.

