# Scripts Rhai

Scripts des automations (action `script`) et code de traitement des sources MQTT (R4 lot 5). Décisions : [ADR 0006](adr/0006-rhai-sandbox.md) (spike), [ADR 0034](adr/0034-rhai-scripts.md) (mise en œuvre).

Les scripts sont écrits en **[Rhai](https://rhai.rs)**, un langage embarqué proche de Rust/JavaScript. Un code **Python** ne se porte pas tel quel : il est refusé à l'enregistrement avec un message clair (`def`, `import`, deux-points en fin de ligne, `context.`…).

## Ce que voit un script
| Nom | Contenu |
|---|---|
| `topic` | topic du message ou de l'état qui a déclenché le script (vide pour un déclenchement horaire ou solaire) |
| `payload` | le message : objet (`payload.temperature`) ou texte ; une clé absente vaut `()` (jamais d'erreur) |
| `device` | l'appareil concerné ou `()` ; champs ci-dessous |

| Champ d'un appareil | Contenu |
|---|---|
| `name`, `id`, `type` | nom, numéro et type (`light`, `cover`, `sensor`…) |
| `state` | dernier état connu (`device.state.position`) |
| `available` | `true` si l'appareil est joignable |
| `topic` | topic de son état (`zigbee2mqtt/Salon`) ; ajouter `"/set"` pour le commander |
| `age_secs` | âge de son dernier message en secondes (nombre décimal, figé au lancement du script), ou `()` s'il n'a jamais parlé |

| Fonction | Effet |
|---|---|
| `publish(topic, message)` | publie sur le broker (QoS 1, sans retenue). Un texte est publié tel quel, une table (`#{ state: "ON" }`) ou un nombre en JSON. Topic sans joker, 256 caractères au plus. Alias : `mqtt_publish` |
| `scene(nom)` | active la scène de ce nom |
| `get_device(nom)` | lit un appareil (mêmes champs que `device`), ou `()` s'il n'existe pas |
| `log_value(cle, valeur)` | historise une valeur de l'appareil concerné, suivie ou non (sans effet s'il n'y a pas d'appareil) |
| `print(texte)`, `log(texte)` | ligne du journal d'exécution (50 lignes, 500 caractères) |

## Règles du bac à sable
- **Aucun effet direct.** `publish`, `scene` et `log_value` sont mis de côté et appliqués **dans l'ordre après la fin du script**, et **seulement s'il se termine sans erreur** (tout ou rien). Au plus 100 effets par exécution.
- **Limites** : 200 000 opérations, 250 ms, 24 niveaux d'appel, expressions de 48 niveaux, chaînes de 16 Ko, tableaux de 2 000 éléments, tables de 500 clés. Pas d'`eval`, pas d'`import`. Une boucle infinie, une récursion sans fin ou une allocation géante est interrompue **sans impact sur le processus** : le serveur continue de répondre, l'erreur est écrite au journal de l'automation.
- Chaque exécution tourne dans un fil à part (une panique y est une erreur).
- Une automation en script est comptée dans le garde-fou de 30 déclenchements par minute.

## Où écrire un script
- **Automation** : `/admin/automations`, action « Script ». **Source MQTT** : `/admin/mqtt-sources`, champ « Code de traitement » (variables `topic` et `payload`, mêmes fonctions ; pas de journal d'exécution, le résultat va au journal du serveur ; un code en échec est mis en sourdine 10 s).
- La syntaxe est vérifiée à l'enregistrement : l'erreur de l'analyseur (ligne, colonne) s'affiche sous le champ.
- **Essai à blanc** (bouton sous la zone de code, administrateurs) : exécute le script avec un topic et un message d'exemple et les vrais états des appareils, et montre ce qu'il aurait fait — **sans rien publier, activer ni enregistrer**.

## Exemples
```rhai
// Allumer le salon au-dessus de 25 degres, et garder la valeur
if payload.temperature > 25.0 {
    publish("zigbee2mqtt/Salon/set", #{ state: "ON" });
}
log_value("temperature_bureau", payload.temperature);
```

```rhai
// Ouvrir le volet seulement si la porte est fermee
let porte = get_device("Porte");
if porte != () && porte.state.contact == true {
    publish("zigbee2mqtt/Volet-salon/set", #{ state: "OPEN" });
}
```

## Les deux scripts du volet SB-est
Les deux scripts `Volet-SB-est` sont **écrits en Rhai** (`crates/dominet-domain/data/volet-sb-est-*.rhai`, déclencheur `device_state` sur `action` de l'interrupteur, `single_left` et `single_right`). Le bouton gauche ouvre, le droit ferme, et un appui pendant le mouvement arrête le volet.

Le relais réel (`Relais-SB-est`) n'a **ni clé `moving` ni état `STOP`** : son `state` se déduit de la position. Il publie environ un message par seconde tant qu'il bouge, puis se tait ; le script le considère donc **en mouvement si son dernier message a moins de 2 secondes** (`age_secs`) :
```rhai
let volet = get_device("Relais-SB-est");
if volet == () {
    throw "appareil Relais-SB-est introuvable";
}
let commande = volet.topic + "/set";
if volet.age_secs != () && volet.age_secs < 2.0 {
    publish(commande, #{ state: "STOP" });
} else {
    publish(commande, #{ state: "OPEN" });   // "CLOSE" pour le bouton droit
}
```

Pour s'en servir : créer deux automations « état d'appareil » sur l'interrupteur (clé `action`, valeur `single_left` puis `single_right`), action « Script », et y coller ce code (en adaptant le nom du relais). Elles remplacent les deux automations simples « ouvrir » et « fermer ».

## Tests
`cargo test -p dominet-automation --lib script` (API, effets dans l'ordre, tout ou rien, **scripts hostiles** : boucles, récursion, chaînes / tableaux / tables géants, `pad`, `eval`, imbrication, effets sans fin ; limite de durée seule ; refus d'un autre langage), `--test engine` (scripts dans le moteur, plugins de la production, code de source mis en sourdine), `-p dominet --test ui script` (essai à blanc, syntaxe, droits), `-p dominet-server --test automations` (de bout en bout sur le broker de dev) ; E2E `automations` (éditeur, essai à blanc, exécution, boucle infinie pendant que le serveur répond).
