# 0029 — Commandes, pages et images des appareils


- Statut : accepté
- Date : 2026-11-02

## Contexte
Les lots 1 à 4 amènent les appareils depuis MQTT en base et en direct. Le lot 5 les rend utilisables : les commander, les voir (liste, détail, widgets), les administrer, leur donner une image, générer leur tableau Grafana. L'implémentation de référence publie n'importe quel JSON
(`device_command`), range tout dans une administration générique et stocke l'image téléversée telle quelle.

## Décisions
- **Commandes** (`dominet_server::control`). Tout compte connecté peut commander (D3 du plan R3 : la famille allume une lampe) ; l'ouverture de l'appairage (`permit_join`), le téléversement d'image, la génération Grafana et la configuration des appareils sont **réservés aux
  administrateurs**. La commande est un **objet JSON non vide**, 1 Ko au plus, 3 niveaux au plus (`dominet_domain::command`) — l'interface n'envoie que des objets plats, Zigbee2MQTT accepte quelques objets imbriqués (`color`), rien d'autre n'a de sens. Elle est publiée en QoS 1, sans
  retenue, sur `<préfixe>/<nom>/set`, pour un appareil **actif** seulement. Sans lien avec le broker (rôle `mqtt` absent, broker coupé) elle est refusée tout de suite (`Unavailable`), jamais mise en attente : une commande rejouée plus tard serait une surprise. L'effet se voit quand
  l'appareil republie son état ; l'interface n'anticipe rien.
- **Widgets d'appareil** (`device_widget`). Le type, les libellés (nom, unité) et les clés masquées viennent de la table des appareils (`DeviceMetaDto`, ne portant que les libellés des clés présentes dans l'état). La présentation automatique suit le **type** quand il est précis,
  sinon l'état (ADR 0025 : une lampe Zigbee2MQTT reste `generic`, donc l'état décide). Les commandes reprennent celles de l'implémentation de référence (`{"state":"ON"}`, `brightness`, `color_temp`, `position`, `OPEN/STOP/CLOSE`, `LOCK/UNLOCK`, consigne `occupied_heating_setpoint` par pas de 0,5,
  `fan_mode`). Boutons actifs si le serveur a un client MQTT **et** que l'appareil est disponible ; sinon désactivés avec l'infobulle « source indisponible ». Le curseur de pourcentage du ventilateur, inventé en R2, est remplacé par les modes de la référence.
- **Pages.** `/automation/` (liste) et `/automation/devices/<id>` (D10) — l'entrée « Appareils » de la navigation amorcée pointe déjà sur `/automation/` ; les deux écritures de l'adresse sont servies. Filtres (groupe, emplacement, type) par un formulaire **GET** : pas d'état
  côté navigateur, l'adresse se partage. Les cartes réutilisent le corps du widget (même code, mêmes commandes, même temps réel). Le détail montre l'identité, les topics, l'historique des 24 dernières heures des clés suivies (courbes SVG calculées côté navigateur, pas de bibliothèque)
  et, pour un administrateur, les outils : image, tableau Grafana, appairage (sur la liste).
- **Administration** : `/admin/devices`, `/admin/device-groups`, `/admin/device-labels` avec le CRUD générique (ADR 0014). Nouveau type de champ `Choice` : liste déroulante dont les choix viennent de la base, fournis par un contexte (`Choices`) — les types `Select` restent statiques
  et traduits. Le widget d'un tableau choisit son appareil dans une liste (D2 de R2) au lieu d'un numéro.
- **Images** (D4 du plan) : PostgreSQL (`device_image`), servies par `/media/devices/<id>` avec `ETag` (`private, no-cache` : revalidée, `304` sans corps). Téléversement par formulaire `multipart` vers `/api/devices/<id>/image` (administrateurs, redirection vers la page) : 5 Mo et 8 000 pixels par
  côté au plus, format reconnu par son contenu (JPEG, PNG, WebP, GIF), puis **décodée, réduite à 400 × 300 au plus et réencodée en JPEG** (la transparence est composée sur du blanc). Ce qui est stocké est donc toujours une image saine et petite, quel que soit le fichier envoyé.
  Crate `image` sans fonctionnalités par défaut (jpeg, png, webp, gif).
- **Tableau Grafana** (`dominet_domain::grafana`, `dominet_server::grafana`) : un panneau par clé suivie, même forme que la référence, écrit **en fichier** dans `GRAFANA_DASHBOARDS_DIR` (jamais par l'API : un tableau chargé depuis un fichier devient « provisionné » et l'API refuse de le
  modifier). Écriture atomique (fichier temporaire puis renommage), version incrémentée, ancien fichier supprimé si l'appareil est renommé, nom de fichier assaini. Sans dossier configuré, la page le dit.
- **Simulateur** (D6) : n'agit plus que sur un appareil existant, avec son vrai nom ; il diffuse sans rien écrire en base. Les scripts E2E qui utilisaient des numéros fictifs créent leurs appareils.

## Écarts avec la référence
1. Commande validée (objet, taille, profondeur) au lieu de tout publier.
2. Image réencodée et réduite au lieu d'être conservée telle quelle ; formats refusés par le contenu et non par l'en-tête `Content-Type` envoyé par le navigateur.
3. L'appairage est réservé aux administrateurs (la référence : tout compte connecté).
4. Les tableaux Grafana lisent `device_state_log` (la référence : `automation_devicestatelog`) : les fichiers déjà écrits par l'implémentation de référence doivent être régénérés à la bascule (R7).
5. Plus de page d'édition séparée : l'édition est dans les pages `/admin/…`.

## Conséquences
- Un navigateur sans WebAssembly ne peut pas commander (les boutons sont des actions de l'interface), mais voit toutes les pages ; le téléversement d'image et le filtrage fonctionnent sans JavaScript.
- Le détail d'un appareil n'affiche que des clés numériques ; l'historique textuel (ex. `action`) reste en base et dans Grafana (R4/R7 si besoin).
- L'état optimiste (l'interface montre la commande avant l'écho de l'appareil) n'existe pas : le prix est un léger délai, le gain est qu'un état affiché est toujours un état réel.
