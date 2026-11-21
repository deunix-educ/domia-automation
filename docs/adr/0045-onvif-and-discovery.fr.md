# 0045 — ONVIF, découverte et ajout des caméras


- Statut : accepté
- Date : 2026-11-07

## Contexte
la référence parle ONVIF avec `onvif-zeep`, découvre les caméras par un petit serveur HTTP séparé en réseau hôte (`onvif_discovery_server`, port 8765) et **crée une fiche pour chaque appareil trouvé**, avant même que l'utilisateur ait choisi. ADR 0008 : client SOAP maison. Plan R6 : D5 (découverte), D1 (aucun contact avec la production).

## Décision
- **Client** (`dominet_surveillance::onvif`) : `GetSystemDateAndTime`, `GetDeviceInformation`, `GetCapabilities`, `GetProfiles`, `GetStreamUri`. Réponses lues sans tenir compte des préfixes de noms (les quatre familles de caméras n'utilisent pas les mêmes). WS-Security `PasswordDigest` avec **correction de l'horloge** de la caméra (lue sans authentification d'abord) ; aucune connexion réutilisée (Thingino la ferme).
- **Les adresses de services gardent l'hôte de la caméra enregistrée** : seuls le port et le chemin annoncés par `GetCapabilities` sont repris. Une caméra derrière une translation d'adresse annonce parfois la sienne ; surtout, une réponse ne doit pas pouvoir nous envoyer vers un autre hôte.
- **Secrets** : `Credentials` et `StreamUri` n'affichent rien dans `Debug`. L'URL de flux de la caméra NVT contient le mot de passe en clair : `mask_secrets` le cache par motif (`user:***@`, `password=***`) **et** par valeur connue (un mot de passe qui contient `_` serait sinon coupé trop tôt) ; seule la forme masquée sort du module.
- **Synchronisation** (`onvif_sync`) : comme la référence, le premier profil est le profil principal et PTZ = présence du service PTZ ; identifiants ONVIF, à défaut ceux du flux. Les résultats sont enregistrés sans toucher aux réglages saisis.
- **Ajout par adresse** (`add_by_ip`) : le port ONVIF est cherché parmi 80, 8080, 8899, 2020, 8000 (liste de la référence + 2020), mais **un port ouvert ne suffit plus** : il doit répondre à `GetSystemDateAndTime`. Une adresse déjà connue renvoie la fiche existante. Avec un identifiant, la caméra est synchronisée aussitôt (un échec n'annule pas l'ajout).
- **Découverte (écart avec la référence)** : les résultats sont **listés avec un bouton « Ajouter »**, au lieu de créer une fiche par appareil. Un appareil déjà enregistré à cette adresse est signalé.
- **Découverte WS-Discovery** : 3 sondes espacées d'une seconde, 6 s d'écoute, interface sortante par la route par défaut, une seule annonce par adresse. Le rôle `discovery` (réseau hôte) et le rôle `web` ne s'appellent pas : ils passent **par PostgreSQL** (`LISTEN`/`NOTIFY`, canaux `onvif_discovery_request` / `onvif_discovery_result`), ce qui remplace le serveur HTTP de la référence sans nouveau port. Le rôle `web` relaie les résultats au sujet WebSocket `discovery` (administrateurs seulement), après avoir marqué les adresses déjà enregistrées. Pas de rôle `discovery` : l'interface le dit au bout de cinq secondes.
- **Pas de découverte par défaut** : `dominet serve` ne lance pas le rôle `discovery` (il exige le réseau hôte) ; `dominet serve --only discovery`.
- **Vérification** : un faux serveur ONVIF vérifie le digest et l'horloge comme une caméra (4 familles de réponses) ; un faux répondeur WS-Discovery répond en unicast sur la boucle locale (la CI ne fait jamais de multicast). Les réponses de `fixtures/onvif/` sont **reconstituées** d'après le spike 6 (aucun secret, aucune adresse réelle) : elles n'ont pas été captées sur les caméras, faute de `deploy/.env.cameras` sur cette machine. Deux tests **ignorés par défaut** les confrontent aux vraies caméras (`real_onvif`, lecture seule) quand le fichier d'accès est présent.

## Conséquences
- Une caméra que le multicast ne voit pas (la Q3R) s'ajoute par son adresse.
- Le PTZ, l'imaging et le night mode `onvif` (lot 5) réutilisent ce client.
- Le rôle `discovery` a besoin d'un accès à la base : il l'utilise comme canal de messages.
