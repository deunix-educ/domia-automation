# Serveur MCP (R11, en cours)

Le **serveur MCP** de DomiNet permet à un assistant (Claude Code, Claude Desktop, ou tout client MCP) de consulter, piloter et configurer l'application avec les mêmes règles que l'interface, **sans jamais voir un secret**. Feuille de route : `ROADMAP-mcp.md` ; décisions : [ADR 0058](adr/0058-mcp-server.md) et [ADR 0059](adr/0059-mcp-access-tokens.md) ; audit : [R11-audit](mcp-audit.fr.md).

!!! warning "État : lot M4d (configuration)"
    Jetons et audit (M1), lecture (M2), pilotage et caméras (M3) et **configuration** (M4 à M4e, seize types) et **administration** (M5) sont en place. Reste la recette (M6).

## Activer
Dans `.env` (le serveur est **désactivé par défaut**) :
```bash
MCP_ENABLED=true
```
Puis recréer le service `dominet` (`make prod-up`). Le point d'entrée est `https://dominet.local/mcp` ; il n'est joignable que par le LAN et le VPN. L'en-tête `Host` doit être `localhost` ou figurer dans `ALLOWED_HOSTS` (protection contre le DNS rebinding). **Sans jeton, personne n'entre.**

## Créer un jeton
Page **Administration › Assistants (MCP)** (`/admin/mcp`) : un nom, les **portées** et une durée de validité (30 à 365 jours : un jeton sans date de fin n'existe pas). La valeur du jeton, `dnm_…`, est **affichée une seule fois**, avec la commande prête à coller dans Claude Code ; seule son empreinte est conservée, elle ne peut pas être relue. Un jeton perdu se révoque et se recrée.

| Portée | Permet |
|---|---|
| `lecture` | consulter appareils, historique, journaux, alertes et configuration (secrets masqués) |
| `pilotage` | commander les appareils, lancer des scènes, acquitter des alertes |
| `configuration` | modifier automations, règles, tableaux de bord et sources |
| `administration` | réglages, caméras et scripts Rhai |

Les portées sont **indépendantes** : `configuration` ne donne pas `lecture`. Prendre un jeton `lecture` pour le diagnostic courant, un jeton plus large seulement le temps d'une modification.

## Brancher Claude Code
```bash
claude mcp add --transport http dominet https://dominet.local/mcp --header "Authorization: Bearer <jeton>"
```
Le poste doit faire confiance à l'autorité de certification de DomiNet (`/dominet-ca.crt`). La commande a été vérifiée avec Claude Code sur la pile de développement (adresse `http://127.0.0.1:8080/mcp`) ; l'essai en `https://dominet.local` sur l'EliteDesk reste à faire (M6). Pour ne brancher l'assistant que dans un dossier, ajouter `--scope local` ; `claude mcp list` doit afficher `dominet … ✔ Connected`, `claude mcp remove dominet` le retire.

## Essai pas à pas (pile de développement)
Un essai complet, sans toucher à la production : la pile de développement est jetable.

1. **Démarrer la pile et le serveur, MCP activé** :
   ```bash
   make dev-up
   cargo leptos build                      # la première fois, ou après une modification
   MCP_ENABLED=true tools/dev-run.sh       # serveur sur http://127.0.0.1:8080, journal « serveur MCP monté sur /mcp »
   ```
2. **Créer un jeton** : se connecter à `http://127.0.0.1:8080` (compte et mot de passe de développement dans `deploy/.env.dev-run`), page **Administration › Assistants (MCP)**, nom `essai`, les quatre portées, 30 jours. Copier la valeur `dnm_…` (affichée une seule fois).
   Sans l'interface, pour un essai seulement : insérer dans la base de développement l'empreinte SHA-256 d'un jeton de son choix (`mcp_token`, colonnes `name`, `token_hash`, `prefix`, `scopes`, `expires_at`).
3. **Brancher Claude Code**, dans un dossier d'essai :
   ```bash
   mkdir -p ~/essai-mcp && cd ~/essai-mcp
   claude mcp add --transport http --scope local dominet http://127.0.0.1:8080/mcp \
     --header "Authorization: Bearer dnm_…"
   claude mcp list                         # dominet … ✔ Connected
   ```
4. **Poser des questions** (`claude` interactif, ou `claude -p "…" --allowedTools "mcp__dominet"`) :
   - *Lecture* : « Donne l'état général du système et dis-moi quels appareils n'ont rien émis depuis 24 h. » (`system_status`, `devices_list`).
   - *Simulation* : « Propose, sans rien écrire, la création d'un emplacement “Essai MCP”. » (`config_schema`, `config_upsert` avec `dry_run`).
   - *Écriture* : « Crée l'emplacement, un appareil “Lampe-essai” de type light rattaché à cet emplacement, puis une automation qui envoie une notification quand un message arrive sur `essai/bouton`. » Relire ensuite avec « Relis l'automation avec config_get ».
   - *Secrets* : « Crée une caméra “Cam essai” avec le mot de passe `…`, puis relis-la. » Le mot de passe ne se relit pas : `has_rtsp_pass: true` seulement.
   - *Garde-fous* : « Supprime l'emplacement sans confirmer. » L'appel est refusé (`confirm = true` obligatoire) ; `dry_run` dit d'abord ce qui dépend de l'élément.
5. **Contrôler** : la page **Administration › Assistants (MCP)** montre le journal d'audit (une ligne par écriture ou refus, jamais de valeur de jeton ni de secret). Les éléments créés se voient dans l'interface.
6. **Nettoyer** : `claude mcp remove dominet`, révoquer le jeton dans la page MCP, arrêter le serveur (Ctrl+C), `make dev-reset` pour repartir d'une base vide.

**Résultat de l'essai du 2026-11-16** (Claude Code, pile de développement) : l'assistant a lu l'état du système, simulé puis réellement créé un emplacement, un appareil, une automation et une caméra, relu l'automation, constaté qu'il ne voyait pas le mot de passe de la caméra, et s'est vu refuser la suppression sans `confirm`. Le journal d'audit contient les quatre écritures et aucun secret. Claude Code affiche des avertissements `unknown format "uint32" ignored in schema` : sans conséquence, ils viennent du schéma des sorties.

## Claude Desktop (à vérifier)
Claude Desktop ne sait pas envoyer un en-tête `Authorization` à un serveur distant déclaré dans l'interface : on passe par le pont `mcp-remote` (Node.js requis), dans `claude_desktop_config.json` :
```json
{ "mcpServers": { "dominet": {
    "command": "npx",
    "args": ["-y", "mcp-remote", "https://dominet.local/mcp", "--header", "Authorization:${DOMINET_AUTH}"],
    "env": { "DOMINET_AUTH": "Bearer dnm_…", "NODE_EXTRA_CA_CERTS": "/chemin/dominet-ca.crt" }
} } }
```
Cette configuration n'a **pas** été essayée ici (pas de Claude Desktop sur le poste de développement) ; elle suit la méthode courante de `mcp-remote`. Un jeton plus étroit (`lecture` seule) est le bon choix pour un client grand public.

## Prompts prêts à l'emploi
Trois modèles, offerts avec la portée `lecture` (ce sont de simples textes : ils ne donnent aucun droit que le jeton n'a pas). Dans Claude Code, ils s'appellent par `/mcp__dominet__diagnostic`, `/mcp__dominet__nouvelle_automation` et `/mcp__dominet__bilan_surveillance`.

| Prompt | Argument | Ce qu'il fait faire à l'assistant |
|---|---|---|
| `diagnostic` | `hours` (12) | état du système, appareils muets, alertes, erreurs récentes, automations en échec, puis une synthèse de 5 lignes |
| `nouvelle_automation` | `need` (obligatoire) | lire les vrais appareils et scènes, lire le schéma, proposer en mots simples, simuler (`dry_run`), créer **seulement après accord** |
| `bilan_surveillance` | `hours` (24) | caméras, détections, enregistrements, énergie, alertes ; ni capture ni mouvement de caméra sans demande |

## Vérifier un serveur en lecture seule
```bash
MCP_TOKEN=dnm_… tools/check-mcp.sh http://127.0.0.1:8080/mcp                   # pile de développement
CACERT=dominet-ca.crt MCP_TOKEN=dnm_… tools/check-mcp.sh https://dominet.local/mcp   # EliteDesk
```
Le script contrôle le refus sans jeton (401), compte les outils et les prompts offerts au jeton et appelle `system_status`. Il n'écrit rien et n'affiche jamais le jeton.

## Recette sur l'EliteDesk (à jouer par vous)
Rien n'est lancé sur la production sans vous. Dans l'ordre :

1. **Activer** : ajouter `MCP_ENABLED=true` au `.env` de l'EliteDesk, puis `make prod-up` (le serveur MCP est désactivé par défaut).
2. **Contrôler le refus** : `curl -s -o /dev/null -w '%{http_code}\n' -X POST --cacert dominet-ca.crt https://dominet.local/mcp` doit répondre **401**.
3. **Créer un jeton `lecture`**, 30 jours, page **Administration › Assistants (MCP)**.
4. **Contrôler** : `CACERT=dominet-ca.crt MCP_TOKEN=dnm_… tools/check-mcp.sh https://dominet.local/mcp`.
5. **Brancher Claude Code** (poste de confiance : `NODE_EXTRA_CA_CERTS=/chemin/dominet-ca.crt` si la CA n'est pas installée) puis `claude mcp add --transport http dominet https://dominet.local/mcp --header "Authorization: Bearer dnm_…"`.
6. **Diagnostic** : lancer `/mcp__dominet__diagnostic`. Comparer avec la page d'accueil et la page Journaux.
7. **Une action réversible** (jeton `pilotage`, créé pour l'occasion) : demander d'allumer puis d'éteindre une lampe ; vérifier les deux lignes du journal d'audit.
8. **Révoquer** les jetons d'essai ; garder `MCP_ENABLED=true` ou le retirer, à votre choix.

Notez ce qui diffère de l'essai en développement : certificat, nom d'hôte (`ALLOWED_HOSTS`), appareils réels.

## Outils disponibles (lecture)
Vingt outils, tous en portée `lecture` ; un assistant ne voit que ceux que son jeton autorise.

| Outil | Rôle |
|---|---|
| `system_status` | version, base de données, appareils actifs et injoignables, alertes en cours, appairage Zigbee, charge de la machine |
| `devices_list` | appareils : joignable, dernière réception, **silence en heures**, lieu, groupe ; filtres (`silent_for_hours`, joignable, type, nom, inactifs) |
| `device_get` | un appareil (nom ou numéro) : état courant, topics MQTT, clés historisées |
| `device_history` | historique d'une clé (température, puissance…) : points, minimum, maximum, moyenne |
| `automations_list` | automations : déclencheur, action, dernier déclenchement, nombre de déclenchements |
| `automation_get` | une automation en détail, **sans le code d'un script** |
| `automation_runs` | journal d'exécution : quand, issue (`ok`, `error`, `skipped`, `limited`) et message ; filtre par automation et par issue |
| `scenes_list` | scènes et ce que chacune envoie à chaque appareil |
| `alerts_list` | alertes (en cours, acquittées, résolues), gravité, valeur déclenchante |
| `cameras_list` | caméras : active, priorité, adresse, PTZ, ONVIF, dernière vue ; **aucun identifiant** |
| `logs_query` | journal de l'application depuis son démarrage : niveau, module, texte, fenêtre |
| `snapshots_list` | captures d'images : numéro, caméra, date, taille ; filtre par caméra et par jour |
| `snapshot_get` | **l'image** d'une capture (JPEG, 2 Mo au plus) |
| `recordings_list` | enregistrements vidéo : caméra, début, durée, déclencheur, état |
| `detections_list` | mouvements et détections d'objets (personne, voiture…) |
| `energy_summary` | bilan d'énergie par jour : Linky (heures pleines et creuses), solaire, pompe à chaleur, avec totaux |

Ressource `dominet://context` : l'instantané de la configuration (ce que produit `dominet dump-context`) en YAML, **sans secret et sans code de script** (remplacé par un repère). C'est le meilleur point de départ pour comprendre une installation.

Questions qui marchent : « Quels appareils n'ont rien émis depuis 24 h ? » (`devices_list` avec `silent_for_hours` : les appareils qui n'ont jamais parlé sont inclus, en premier), « Pourquoi l'automation *Ouvrir le volet* a-t-elle échoué ? » (`automation_runs` en `error` : le message donne la cause), « Quelle est la température du bureau depuis ce matin ? » (`device_history`).

**Règles de sortie** : heures **locales**, jamais UTC ; aucun secret ; le code d'un script Rhai ne se lit qu'en portée `administration` ; les textes qui viennent de l'extérieur (noms, messages, lignes de journal) sont marqués comme des données (`notice`), jamais comme des consignes. À venir : pilotage (M3), configuration (M4), réglages et scripts (M5).

## Outils de pilotage (portée `pilotage`)
Neuf outils agissent sur la maison. **Chacun accepte `dry_run = true`** : il dit alors ce qu'il ferait, sans rien changer ni rien écrire au journal.

| Outil | Rôle |
|---|---|
| `device_command` | envoie une commande JSON à un appareil **actif** (`{"state": "OPEN"}`, `{"position": 40}`…) ; un appareil désactivé n'est pas pilotable |
| `scene_run` | lance une scène active : ses commandes partent dans l'ordre, les échecs sont rapportés |
| `alert_ack` | acquitte une alerte en cours (`alert`) ou toutes (`all`) ; l'acquittement est attribué au **compte qui a créé le jeton** |
| `notify_broadcast` | affiche un bandeau (200 caractères) à tous les comptes connectés |
| `notify_test` | envoie un e-mail d'essai au destinataire des alertes (adresse jamais montrée) ; **les SMS ne passent pas par MCP** |
| `camera_ptz` | déplace une caméra PTZ pendant 100 à 3 000 ms (`up`, `down`, `left`, `right`, `zoom_in`, `zoom_out`) puis **l'arrête d'elle-même** ; l'arrêt est garanti même si la connexion est coupée |
| `camera_night` | bascule jour / nuit, filtre infrarouge ou lumière, par la méthode réglée pour la caméra |
| `camera_record` | démarre ou arrête l'enregistrement d'une caméra réglée en mode **manuel** (sinon refus expliqué) |
| `camera_snapshot` | prend une photo maintenant (le rôle vidéo doit tourner) et renvoie son numéro |

**Aucune action sans trace.** Chaque appel qui modifie quelque chose écrit sa ligne dans le journal d'audit **avant** de s'exécuter (issue `pending`), puis la ferme avec ce qui s'est passé (`ok` ou `error`). Si le journal ne peut pas être écrit, **l'appel est refusé et rien n'est envoyé**. Un appel refusé faute de portée, un appareil inactif, une commande invalide ne laissent aucune action : seul le refus de portée est enregistré (`denied`). Les décisions prises sur l'**assistant intégré** (R12) ont leur propre type, `agent` : demande en clair, action confirmée ou refusée, et de quoi la défaire (`ROADMAP-agent.md` §14).

## Configuration (portées `lecture` et `configuration`)
Sept outils lisent et écrivent la configuration, pour seize types : **emplacement** (`location`), **scène** (`scene`), **automation** (`automation`), **règle d'alerte** (`alert_rule`), **groupe d'appareils** (`device_group`), **libellé de clé d'état** (`device_label`), **source MQTT** (`mqtt_source`) **station météo** (`weather_station`), **règle Prometheus** (`prometheus_rule`) **règle Grafana** (`grafana_rule`), **tableau de bord** (`dashboard`, nommé `propriétaire/nom`) **widget** (`widget`, placé sur la grille de 12 colonnes), **appareil** (`device` : ses paramètres, jamais son état), **entrée de la barre de navigation** (`navbar`, icône Lucide vérifiée) **caméra** (`camera`, portée `administration`) et **détecteur** d'objets (`detector` : un seul élément, qui se modifie sans se créer ni se supprimer). Un widget qui sortirait de la grille ou en chevaucherait un autre est refusé dès le `dry_run` ; son contenu texte est assaini ; supprimer un tableau supprime ses widgets (le `dry_run` en donne le nombre) ; supprimer un appareil supprime son historique et laisse ses widgets sans appareil. Enregistrer ou supprimer une règle réécrit `rules.yml` ou `rules.yaml` et recharge le service (au mieux : un rechargement en échec n'annule pas l'enregistrement, la page des règles en montre l'état).

| Outil | Portée | Rôle |
|---|---|---|
| `config_kinds` | lecture | les types configurables, avec la portée de lecture et d'écriture |
| `config_schema` | lecture | ce que `config_upsert` accepte pour un type : schéma JSON, **exemple complet** et notes (clés de chaque déclencheur et action) |
| `config_list`, `config_get` | lecture | les éléments d'un type, **dans la forme exacte que `config_upsert` accepte** : lire, changer un champ, écrire |
| `config_upsert` | configuration | crée (sans `id`) ou **remplace** (avec `id`) un élément ; `dry_run` montre les champs qui changeraient |
| `config_delete` | configuration | supprime ; **`confirm = true` obligatoire**, `dry_run` dit ce qui en dépend |
| `zigbee_pairing` | configuration | ouvre (254 s au plus) ou ferme l'appairage Zigbee |

**Les mêmes contrôles que l'interface**, parce que le chemin est le même : un service partagé (`dominet_server::config_service`) valide le brouillon du domaine, vérifie qu'un **script Rhai compile** et que la **scène visée existe**, traduit les conflits de nom en erreurs de champ, puis recharge le moteur d'automations, le planificateur ou l'évaluation des alertes. Un `dry_run` annonce donc exactement les refus d'une écriture réelle. Les champs inconnus sont refusés (une faute de frappe n'est pas ignorée en silence).

**Le code Rhai** (script d'une automation, `handler_code` d'une source MQTT) et **la requête SQL d'une règle Grafana** (toute source autre que `prometheus`) : jamais lisible (`script` vaut `null`, `has_script` l'indique). L'écrire, le changer ou le retirer exige **en plus** la portée `administration`. Une mise à jour qui **ne mentionne pas** `script` **conserve** le code existant : relire une automation, modifier un champ et la réécrire n'efface donc jamais son code.

Exemple, de bout en bout : « ferme les volets au coucher du soleil » → `config_schema` (type `automation`) → `config_upsert` avec `dry_run`, puis sans. L'instantané `dominet://context` avant et après ne diffère que de cette automation (c'est un test).

## Administration (portée `administration`)

La portée `administration` donne accès à ce qui est sensible : réglages, code des scripts, caméras. Elle est indépendante des autres et se donne rarement.

- **Réglages** : `settings_list` (la valeur d'un secret n'est **jamais** donnée, seulement `is_set`) et `setting_set` (valeur validée selon le type du réglage ; un secret est chiffré, `dry_run` dit si la valeur serait acceptée).
- **Scripts Rhai** : `script_get` lit le code d'une automation (action `script`) ou d'une source MQTT ; `script_check` le compile et, avec un `topic`, l'essaie sur un message d'exemple dans le vrai bac à sable, sans rien publier ; `script_set` le remplace après les mêmes contrôles que l'interface. Les autres outils ne montrent jamais ce code.
- **Caméras** : le type `camera` de `config_*` (lecture, écriture et suppression exigent `administration`, car on y trouve adresses et noms d'utilisateur). Les mots de passe (`rtsp_pass`, `onvif_pass`, `web_pass`) et la clé d'API (`web_api_key`) **s'écrivent et ne se lisent jamais** : `config_get` dit seulement `has_rtsp_pass`… ; un champ omis garde le secret en place. `cameras_sync` met MediaMTX à jour (`target = mediamtx`) ou apprend d'une caméra ce qu'elle annonce (`target = onvif`).

Aucun secret ne figure dans une réponse ni dans le journal d'audit : un test balaie les sorties.

## Sécurité
- **Révocation immédiate** : un jeton révoqué ou expiré est refusé dès l'appel suivant.
- **Limite de débit** par jeton (`MCP_RATE_LIMIT_PER_MIN`, 120 par défaut) : au-delà, **429** avec `Retry-After`.
- **Journal d'audit** (même page) : création et révocation des jetons, appels refusés et, dès le lot M3, appels qui modifient quelque chose. Les secrets y sont masqués ; purge au bout de `MCP_AUDIT_RETENTION_DAYS` (365 jours par défaut).
- Sans jeton ou avec un jeton faux, périmé ou révoqué, la réponse est **401** (`WWW-Authenticate: Bearer`) ; avec un `Host` inconnu, **403** ; serveur désactivé, **404**. Ni l'en-tête `Authorization` ni la valeur d'un jeton ne sont journalisés.

## Tests
`cargo test -p dominet-mcp` : un vrai client MCP (`rmcp`) sur un serveur réel et la base de test, **chaque outil de lecture sur des données réelles** (`--test read`), jeton lu en base, révocation, expiration, portée refusée et auditée, limite de débit, protection du `Host`, client de la révision précédente du protocole, serveur désactivé ; `--test coverage` : chaque fonction serveur de l'interface doit être classée dans `tests/mcp/coverage.tsv`. Dépôts : `cargo test -p dominet-store --test mcp`.
