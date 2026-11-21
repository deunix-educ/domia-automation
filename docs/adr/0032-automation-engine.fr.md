# 0032 — Moteur d'automations (déclencheurs `mqtt_message` et `device_state`)


- Statut : accepté
- Date : 2026-11-02

## Contexte
Le plan R4 (D1 à D3) a établi, en lisant l'implémentation de référence, que `device_state` n'y est jamais évalué, que `mqtt_message` ne l'est que pour les états Zigbee2MQTT et que `mqtt_publish` ne publie rien (client absent). On implémente **l'intention** documentée, en gardant à l'identique ce qui marche dans la référence (jokers de topic, filtre de
payload, condition). Le moteur doit suivre le débit de l'ingestion (2 000 messages/s en R3) sans la ralentir.

## Décisions
- **Un moteur, une file.** L'ingestion (`Sink`) annonce deux choses : *tout message* utile (`message`) et *tout état d'appareil* (`device_state`). Une `EngineHandle` écarte **à la source** ce qu'aucune automation n'attend (lecture d'une copie mémoire des automations actives, sans accès à la base), et empile le reste dans une file bornée (1 024).
  Un seul `Engine::run` consomme la file, événement par événement, dans l'ordre : jamais deux automations en même temps, jamais de course sur les compteurs. File pleine : l'événement est écarté et compté (jamais d'attente côté ingestion).
- **Quels messages déclenchent `mqtt_message`.** Les états et disponibilités d'appareils et **tous les messages hors du préfixe Zigbee2MQTT** (parmi eux ceux d'un topic sans source MQTT : le client s'abonne tout seul au topic de chaque automation `mqtt_message` active, QoS 1). Pas les messages `bridge/…` ni les commandes `…/set` / `…/get` :
  une automation qui commande un appareil se verrait passer sa propre commande.
- **Sens de `device_state`.** Se déclenche pour **chaque message d'état** de l'appareil dont `état[clé]` égale `value`, par la forme texte des scalaires (`28` = `"28"`) ou numériquement (`28.0` = `28`) ; valeurs non scalaires jamais. Les touches de télécommande sont des événements (`action: 1_single`) : un état « ON » qui se répète redéclenche à chaque message.
- **Rafales : les états regroupés restent visibles.** L'ingestion (ADR 0027) ne garde que le dernier état d'un appareil reçu dans la même seconde. Une télécommande envoie `{"action":"1_single"}` puis `{"action":""}` à quelques millisecondes d'écart : le premier serait perdu. Le `Sink` reçoit donc aussi `state_skipped` pour chaque état regroupé
  (pas enregistré, pas diffusé, mais vu des automations, à sa place dans l'ordre). Trouvé par un test intermittent du parcours de la télécommande de la production.
- **Égalités de la référence.** `1 == 1.0`, `True == 1`, égalité récursive des listes et objets ; une clé absente du payload vaut `None` pour le filtre (un filtre `null` la laisse passer) ; correspondance de topic (`#` accepte la fin de n'importe quel niveau). Vérifié contre **l'implémentation de référence** sur 396 cas.
- **Garde-fou anti-boucle (D3).** Au plus **30 déclenchements par automation et par minute** (fenêtre glissante) ; les suivants sont ignorés et le dépassement est inscrit **une fois par minute** au journal (`limited`).
- **Effets par un port.** `Executor` (publier, activer une scène, notifier) : le serveur l'implémente avec le client MQTT du rôle `mqtt` (publication QoS 1, sans retenue), `scenes::activate_by_id` et le sujet temps réel `notifications`. Les tests le simulent. Une scène activée avec un échec, une scène introuvable, une publication refusée sont des **échecs** (`error`) au journal.
- **Journal et compteurs.** Chaque déclenchement qui a passé filtre et condition écrit une ligne `automation_run` (cause, statut, durée, détail) et incrémente `trigger_count` / `last_triggered` — échecs et actions sans effet compris (la référence ne comptait pas les exceptions ; sans importance en pratique). Un filtre ou une condition faux n'écrit rien (le journal serait noyé).
  Purge à 30 jours, une fois par jour.
- **A chaud.** Toute modification (administration) réveille la boucle du rôle `mqtt` (`automations_changed`), qui recharge le moteur et recalcule les abonnements ; relecture de sécurité toutes les 60 s.
- **Scripts et planification.** Une action `script` est journalisée « non exécutée » jusqu'au lot 5 ; `schedule` et `solar` sont enregistrables mais n'ont pas de planificateur avant le lot 4.

## Conséquences
- À la bascule, les cinq automations « inertes » de la production (trois pour la télécommande, deux plugins) deviennent actives : la liste figure dans la synthèse R4.
- Le moteur n'attend jamais l'ingestion et inversement ; sa seule contrainte est la file bornée (jamais atteinte en pratique : une automation dure quelques millisecondes).
- Les écarts avec la référence sont tous voulus (D1, D2, D3) et testés.
