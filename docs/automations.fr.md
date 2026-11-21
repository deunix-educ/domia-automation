# Automations

Moteur et administration des automations (R4 lot 3). Décisions : [ADR 0031](adr/0031-automation-schema.md) (schéma, domaine), [ADR 0032](adr/0032-automation-engine.md) (moteur), plan R4 (D1 à D3).

Une automation a un **déclencheur**, une **condition** facultative et une **action**. Elle est créée, modifiée, activée / désactivée et supprimée par les administrateurs, dans **`/admin/automations`**.

## Déclencheurs
| Type | Se déclenche quand | État |
|---|---|---|
| `device_state` | un **message d'état** de l'appareil contient la clé avec la valeur donnée (`28` = `"28"`) | actif |
| `mqtt_message` | un message arrive sur un topic correspondant au filtre (`+`, `#`) et son contenu contient le **filtre de message** (objet JSON, égalité des valeurs) | actif |
| `schedule` | l'expression cron à cinq champs (heure locale, réglage `TZ`) | actif (planificateur) |
| `solar` | un des neuf événements solaires au lieu donné (latitude, longitude ; virgule décimale admise) | actif (planificateur) |

- **Condition** (facultative) : si le message contient la clé, elle doit valoir la valeur ; sinon la condition est vraie. Les valeurs se saisissent comme du JSON (`100`, `true`, `"ON"`) ou du texte simple (`ON`).
- Un `device_state` se déclenche **à chaque message d'état** qui convient : un état « ON » qui se répète redéclenche. Les touches de télécommande (`action: 1_single`) sont des événements.
- Un `mqtt_message` n'attend ni les messages `bridge/…` ni les commandes `…/set` / `…/get`. **Le client MQTT s'abonne tout seul au topic de chaque automation `mqtt_message` active** : pas de source à créer.
- Un état regroupé par l'ingestion (deux états du même appareil dans la même seconde) reste visible des automations : une télécommande qui envoie `action: 1_single` puis `action: ""` ne perd pas sa touche.

## Actions
| Type | Effet |
|---|---|
| `mqtt_publish` | publie l'objet JSON sur le topic (QoS 1, sans retenue) |
| `scene` | active la scène (les commandes partent dans l'ordre, voir [scenes.md](scenes.fr.md)) ; la scène doit exister à l'enregistrement |
| `notification` | journal + bandeau dans l'interface de tous les comptes connectés **+ e-mail** au destinataire des alertes, et **SMS Free Mobile si la case « SMS » est cochée** (le message par défaut est « Automation <nom> déclenchée » ; R5 lot 6, [ADR 0042](adr/0042-alerts-on-real-channels.md)) |
| `script` | script Rhai exécuté dans un bac à sable, voir [scripts.md](scripts.fr.md) ; ses effets (publier, scène, historique) sont appliqués après coup, seulement s'il se termine sans erreur |

## Planificateur
Les automations `schedule` et `solar` sont exécutées par le rôle `scheduler` ([ADR 0033](adr/0033-scheduler.md)), présent par défaut dans `dominet serve`. Chacune a un travail dans `schedule_job` ; la colonne **Prochain passage** de `/admin/automations` le montre, en **heure locale du serveur** (`TZ`, `Europe/Paris` par défaut), comme toutes les dates de l'interface ; l'API REST reste en UTC. Modifier, désactiver ou supprimer l'automation met le travail à jour tout de suite.
- **Pas de rattrapage** : un passage manqué (serveur arrêté) n'est pas rejoué, sauf s'il est découvert moins de 60 s après son heure.
- **Cron** : cinq champs, dimanche = 0, jour du mois **et** jour de la semaine ; heure locale avec changements d'heure (heure inexistante : décalée d'une heure ; heure répétée : une seule fois).
- **Soleil** : lever, coucher, midi solaire, aubes et crépuscules civil / nautique / astronomique, calculés sans bibliothèque (écart ≤ 13 s avec `ephem` sur nos 1 036 cas). Au-delà des cercles polaires, sans lever ni coucher, l'automation attend le jour où ils reviennent.
- Le travail système **`compute_daily_energy`** (00:05 locale) existe déjà ; son calcul arrive avec le lot 7.

## Garde-fou et journal
- **30 déclenchements par automation et par minute au plus** (anti-boucle) ; au-delà ils sont ignorés et le dépassement est inscrit une fois par minute au journal (`limité`).
- Le **journal** (sous le tableau de `/admin/automations`, 50 dernières lignes) donne la cause, le résultat (`ok`, `échec`, `sans effet`, `limité`), le détail ; le tableau montre le nombre de déclenchements et le dernier. Un filtre ou une condition faux n'écrit rien. Conservation : 30 jours.
- Une modification est prise en compte **tout de suite** (le moteur et les abonnements sont rechargés), sans redémarrage.

## Comportements notables
- `device_state` fonctionne ; `mqtt_publish` publie vraiment ; `mqtt_message` voit tous les messages utiles. `schedule` et `solar` n'ont besoin ni de Redis ni d'un planificateur externe.
- Anti-boucle et journal d'exécution.
- **À la bascule**, les cinq automations « inertes » de la production s'activeront : trois pour la télécommande (`Tele-commande` commandera réellement `Volet-salon`) et deux plugins `Volet-SB-est` (réécrits en Rhai, voir [scripts.md](scripts.fr.md)).

## Pour les développeurs
- `dominet_automation::engine` : `Engine`, `EngineHandle`, `Executor`, `fires_on` (pur), `topic_matches_py`. Le serveur branche le moteur dans `dominet_server::mqtt::spawn` (`ServerExecutor` dans `dominet_server::automation`).
- Domaine : `loose_equal`, `payload_filter_holds`, `device_state_holds`, `value_from_text` (`dominet_domain::automation`).
- Fonctions serveur : `automations/{list,save,delete,runs}` (administrateurs).

## Tests
`cargo test -p dominet-domain --lib automation` (égalités, filtres, sens de `device_state`), `-p dominet-automation --test matching` (**396 cas comparés à des valeurs de référence figées**), `--test engine` (déclencheurs, condition, actions, journal, garde-fou, file, **automations de la production**), `--test hooks` (messages annoncés, états regroupés),
`-p dominet-automation --test schedule` et `-p dominet-domain --test schedule_oracle` (**cron et soleil comparés à des valeurs de référence figées**), `--test scheduler` (synchronisation, passages à l'heure, pas de rattrapage, échecs, les deux automations solaires de la production), `-p dominet-server --test automations` (de bout en bout sur le broker de dev : la télécommande de la production déplace le volet, le coucher / lever du soleil de la production active ses scènes, topic sans source, scène, désactivation à chaud), `-p dominet --test ui automation` (droits, validation champ par champ) ; E2E `automations`.
