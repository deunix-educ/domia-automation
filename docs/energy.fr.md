# Énergie

Bilan d'énergie journalier, page et API (R4 lot 7). Décisions : [ADR 0036](adr/0036-energy.md), plan R4 (D9).

Chaque nuit à **00:05** (heure locale, réglage `TZ`), le travail système `compute_daily_energy` du [planificateur](automations.fr.md) calcule le bilan de **la veille** (jour local) à partir de l'historique des compteurs et l'enregistre (un jour recalculé est remplacé).

| Colonne du bilan | Calcul (de référence) |
|---|---|
| Linky heures pleines / creuses (Wh) | `max − min` du jour des compteurs cumulatifs `HCHP` / `HCHC`, tronqué vers zéro, jamais négatif |
| Production solaire (Wh) | maximum du jour de `qdy` (kWh, remis à zéro chaque nuit) × 1000, tronqué |
| Pompe à chaleur (Wh) | `max − min` de `energy_a` + `max − min` de `energy_b` |
| **Bilan net** | production − (Linky + PAC) : positif = surplus |

Le jour est le jour **local** : 23 h le jour du passage à l'heure d'été, 25 h celui du retour. Un appareil introuvable compte zéro.

## Mise en route
1. Les trois appareils doivent exister (leurs noms : `ENERGY_LINKY_DEVICE`, `ENERGY_SOLAR_DEVICE`, `ENERGY_PAC_DEVICE`, défauts `consommation`, `prod`, `PAC-CE`) : ils viennent des sources MQTT Linky / SolarMax / pompe à chaleur.
2. `dominet seed-energy` (option `--dry-run`) ajoute aux **clés suivies** de ces appareils celles dont le calcul a besoin (`HCHP`, `HCHC`, `qdy`, `energy_a`, `energy_b`) et les puissances du tableau Grafana « Énergie » (`PAPP`, `pac`, `power_ab`) ; sans historique, le bilan reste à zéro. Il ne retire rien.
3. `dominet aggregate-energy --days 30` (ou `--date 2026-06-01`) calcule des jours passés ; ensuite, la nuit s'en charge.

## Page et API
- **`/automation/energy`** (lien « Énergie » sur la page des appareils) : quatre cartes **en direct** (production solaire en W et production du jour en kWh ; consommation Linky en VA et tarif ; PAC en W ; **bilan net** en W, « surplus » ou « déficit »), un histogramme (solaire, Linky, PAC en barres, bilan net en courbe) calculé par le serveur en **SVG** — aucune bibliothèque JavaScript —, les plages 7 / 14 / 30 / 90 jours, et le tableau (le plus récent d'abord).
- **`GET /api/energy/?days=N`** (compte connecté ; 30 par défaut, 365 au plus) : `{"data": [{"date", "linky_hp_wh", "linky_hc_wh", "linky_total_wh", "solar_wh", "pac_wh", "net_wh"}]}`, du plus ancien au plus récent.

## Comportements notables
- Les `N` **derniers** jours (et non les `N` premiers enregistrés, ce qui figerait la page au bout de trente jours).
- La production du jour s'affiche en **kWh**.
- Le graphique est un SVG dessiné par le serveur (aucun script externe) ; l'API est à `/api/energy/`.

## Tableau Grafana
Un tableau « DomiNet — Énergie » reprend ce bilan dans Grafana, avec le taux d'**autoconsommation** : voir [observabilité](observability.fr.md).

## Tests
`cargo test -p dominet-automation --test energy` (**13 jours comparés à des valeurs de référence figées** : nuits de changement d'heure, bordures de jour local, remise à zéro, jour sans donnée, troncature `int(1.005 × 1000)`), `-p dominet-domain --lib energy` (SVG sans script ni `style`, graduations), `-p dominet-server --test energy` (`seed-energy`, travail de 00:05) et `--lib` (paramètre `days`), `-p dominet --test ui energy` (API, page), E2E `energy` (le vrai planificateur calcule hier ; page, plages, API, direct sans recharger).
