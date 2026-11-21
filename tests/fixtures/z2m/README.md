# Fixtures Zigbee2MQTT

Format : une ligne JSON par message `{"topic": "...", "payload": <objet|chaine>, "retain": bool}`.

**`session-r3.jsonl`** (rejeu de R3) : session riche, avec des opérations de test (`track`, `activate`, `deactivate`) et un temps virtuel `t` (secondes) ; son résultat attendu `session-r3.golden.json` est un résultat de référence figé (voir `docs/pitfalls.md`, `make oracle`), voir l'ADR 0027.

**`session-basic.jsonl` — Statut : synthétique.** Elles reproduisent la forme documentée de Zigbee2MQTT et ce que lit
l'ingestion. Les captures réelles (anonymisées) les remplaceront pour les
tests de contrat de R3. Aucune donnée de production ici.
