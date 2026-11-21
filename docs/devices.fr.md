# Appareils

Pages, commandes et administration des appareils (R3 lot 5). Décisions : [ADR 0025](adr/0025-device-schema.md) (schéma), [ADR 0027](adr/0027-zigbee2mqtt-ingestion.md) et [ADR 0028](adr/0028-mqtt-sources-and-raw-archive.md) (d'où viennent les appareils), [ADR 0029](adr/0029-device-commands-and-pages.md) (ce document).

## Pour tous les comptes
- **`/automation/`** (entrée « Appareils » de la navigation) : les appareils actifs, en cartes qui suivent l'état **en direct**. Filtres par groupe, emplacement et type (formulaire : l'adresse se partage).
- **Commandes** sur les cartes et sur les widgets du tableau de bord : ON/OFF, luminosité, température de couleur, volets (ouvrir, stop, fermer, position), consigne de thermostat (pas de 0,5 °C), modes du ventilateur, verrou. Elles sont publiées sur
  `<préfixe>/<nom>/set` ; l'appareil confirme en republiant son état. Boutons **désactivés** (« source indisponible ») si l'appareil est hors ligne ou si le serveur n'est pas relié au broker.
- **`/automation/scenes`** : les scènes et leur activation, voir [scenes.md](scenes.fr.md).
- **`/automation/devices/<id>`** : identité (type, groupe, emplacement, adresse IEEE, modèle), topics d'état et de commande, dernier message, **courbes des dernières 24 h** des clés suivies.

## Pour les administrateurs
- **Appairage** Zigbee2MQTT : bouton sur `/automation/` (publie `{"time": 254}` pour ouvrir, `{"time": 0}` pour fermer, sur `<préfixe>/bridge/request/permit_join` ; format de Zigbee2MQTT 2.x, l'appairage se referme seul après 254 s).
- **`/admin/devices`**, **`/admin/device-groups`**, **`/admin/device-labels`** : appareils (type, groupe, emplacement, clés suivies et masquées…), groupes, libellés des clés d'état (nom affiché, icône, unité ; `dominet seed-labels` crée les usuels). Les clés suivies sont séparées
  par des virgules ; ce que Zigbee2MQTT découvre (adresse IEEE, modèle) et l'état ne se modifient pas à la main.
- **Image** d'un appareil (page de l'appareil) : JPEG, PNG, WebP ou GIF jusqu'à 5 Mo ; elle est réduite à 400 × 300 au plus et stockée en base.
- **Tableau Grafana** d'un appareil (bouton sur sa page) : un panneau par clé suivie, écrit dans `GRAFANA_DASHBOARDS_DIR` (le dossier surveillé par Grafana). 
- **Widgets** (`/admin/widgets`) : l'appareil se choisit dans une liste.
- **Simulateur** (`/admin/settings`) : injecte un état pour un appareil **existant** (diffusion seulement, rien n'est écrit en base).

## Tests
`cargo test -p dominet --test ui` (pages, droits, commandes, administration, Grafana), `-p dominet-server --test media` (images), `--test mqtt_role` (commandes publiées sur le broker de dev), `-p dominet-server --lib media grafana`, `-p dominet-domain` (validation des commandes,
libellés, tableau Grafana) ; E2E `devices` (parcours complet dans Firefox : administration, cartes, commandes reçues par le broker, image, courbe, Grafana).
