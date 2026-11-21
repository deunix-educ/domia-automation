# 0036 — Bilan d'énergie journalier


- Statut : accepté
- Date : 2026-11-03

## Contexte
L'implémentation de référence agrège chaque nuit (un planificateur externe, 00:05) l'historique des compteurs en un bilan par jour (`EnergyDailyLog`) : Linky heures pleines et creuses, production solaire, pompe à chaleur. Une page affiche les 30 derniers bilans (graphique Chart.js chargé depuis un CDN), l'état en direct, et une API JSON. Le plan R4 (D9) demande le même calcul, vérifié contre la référence, un graphique sans bibliothèque JavaScript, et de quoi mettre les appareils en état de produire l'historique.

## Décisions
- **Même calcul que `compute_daily_energy`**, exécuté par l'implémentation de référence comme oracle :
  - le jour est le **jour local** (du minuit local au minuit local suivant : 23 h le jour du passage à l'heure d'été, 25 h le jour du retour) — `start + timedelta(days=1)` d'un `datetime` zoneinfo est une addition murale ;
  - compteurs cumulatifs (`HCHP`, `HCHC`, `energy_a`, `energy_b`) : maximum − minimum du jour, **tronqué vers zéro**, jamais négatif ; les valeurs texte sont ignorées ;
  - production = maximum de `qdy` (kWh, remis à zéro chaque jour) × 1000, tronqué (`int(1.005 * 1000)` vaut 1004 dans la référence : même flottant, même résultat) ;
  - un appareil introuvable compte zéro (avertissement).
  13 jours (dont les nuits de changement d'heure, la dernière seconde et la première seconde d'un jour local, une valeur remise à zéro, une journée sans donnée) sont identiques.
- **Appareils** : `ENERGY_LINKY_DEVICE`, `ENERGY_SOLAR_DEVICE`, `ENERGY_PAC_DEVICE` (défauts `consommation`, `prod`, `PAC-CE`, les noms de la production), lus par la configuration. Le calcul vit dans `dominet_automation::energy`, générique sur les dépôts.
- **Qui le lance** : le travail système `compute_daily_energy` du planificateur (00:05 locale, ADR 0033), qui calcule **hier** (jour local) ou `args.date` ; `dominet aggregate-energy [--days N | --date AAAA-MM-JJ]` pour rattraper. Recalculer remplace (upsert).
- **`dominet seed-energy`** ajoute (sans rien retirer) aux clés suivies des trois appareils celles dont le calcul a besoin : sans historique de ces clés, le bilan reste à zéro et rien ne le dit. Un appareil absent est signalé, pas créé.
- **Écart voulu avec la référence : les `N` derniers jours.** `order_by("date")[:30]` renvoyait les 30 premiers bilans enregistrés : la page se figeait au bout de trente jours. `EnergyRepo::recent(n)` rend les n derniers, du plus ancien au plus récent.
- **API** `GET /api/energy/?days=N` (compte connecté ; 30 par défaut, 365 au plus, illisible = 30) au format de la référence (`{"data": [...]}` avec `linky_total_wh` et `net_wh` calculés). L'URL change (`/automation/energy/api/` → `/api/energy/`), le format non.
- **Page** `/automation/energy` : quatre cartes en direct (production solaire `pac` et `qdy` du jour, consommation Linky `PAPP` et `PTEC`, PAC `power_ab`, bilan net = production − consommation − PAC avec « surplus » / « déficit ») alimentées par les sujets `device:<id>` ; plages 7 / 14 / 30 / 90 jours ; tableau, plus récent d'abord. La production du jour s'affiche en **kWh** (la référence écrivait « Wh » devant une valeur en kWh).
- **Graphique en SVG calculé côté serveur** (`dominet_domain::energy::chart_svg`, testé) : barres solaire / Linky / PAC et courbe du bilan net aux couleurs de la référence, graduations « rondes » (1, 2, 5 × 10ⁿ), axe négatif quand un jour est déficitaire, info-bulles (`<title>`). Aucun script, aucun attribut `style` (CSP) : insertion par `inner_html` d'un texte entièrement produit par le serveur à partir de nombres et de dates.

## Conséquences
- Sans suivi des clés (`seed-energy`) ni historique, la page l'indique et propose les deux commandes.
- Le bilan du jour en cours n'existe qu'après 00:05 : la page ne montre « aujourd'hui » qu'en direct (comme la référence).
- À la bascule (R8), l'historique de la production sera importé ; le bilan pourra être recalculé pour n'importe quel jour par `aggregate-energy`.
