# Energy

Daily energy balance, page and API (R4 lot 7). Decisions: [ADR 0036](adr/0036-energy.md), R4 plan (D9).

Every night at **00:05** (local time, `TZ` setting), the system job `compute_daily_energy` of the [scheduler](automations.md) computes **yesterday's** balance (local day) from the counters' history and stores it (a recomputed day is replaced).

| Balance column | Computation (reference) |
|---|---|
| Linky peak / off-peak (Wh) | `max − min` of the day for the cumulative counters `HCHP` / `HCHC`, truncated toward zero, never negative |
| Solar production (Wh) | day maximum of `qdy` (kWh, reset every night) × 1000, truncated |
| Heat pump (Wh) | `max − min` of `energy_a` + `max − min` of `energy_b` |
| **Net balance** | production − (Linky + heat pump): positive = surplus |

The day is the **local** day: 23 h on the day of the switch to summer time, 25 h on the day of the return. A missing device counts as zero.

## Getting started
1. The three devices must exist (names: `ENERGY_LINKY_DEVICE`, `ENERGY_SOLAR_DEVICE`, `ENERGY_PAC_DEVICE`, defaults `consommation`, `prod`, `PAC-CE`): they come from the Linky / SolarMax / heat pump MQTT sources.
2. `dominet seed-energy` (option `--dry-run`) adds to these devices' **tracked keys** the ones the computation needs (`HCHP`, `HCHC`, `qdy`, `energy_a`, `energy_b`) and the powers of the Grafana "Energy" dashboard (`PAPP`, `pac`, `power_ab`); without history the balance stays at zero. It removes nothing.
3. `dominet aggregate-energy --days 30` (or `--date 2026-06-01`) computes past days; after that the night job does it.

## Page and API
- **`/automation/energy`** (the "Energy" link on the devices page): four **live** cards (solar production in W and today's production in kWh; Linky consumption in VA and tariff; heat pump in W; **net balance** in W, "surplus" or "deficit"), a histogram (solar, Linky, heat pump as bars, net balance as a line) computed by the server as **SVG** — no JavaScript library —, the 7 / 14 / 30 / 90 day ranges, and the table (most recent first).
- **`GET /api/energy/?days=N`** (signed-in account; 30 by default, 365 at most): `{"data": [{"date", "linky_hp_wh", "linky_hc_wh", "linky_total_wh", "solar_wh", "pac_wh", "net_wh"}]}`, oldest first.

## Notable behaviors
- The **last** `N` days (not the first `N` recorded, which would freeze the page after thirty days).
- Today's production is shown in **kWh**.
- The chart is a server-drawn SVG (no external script); the API is at `/api/energy/`.

## Grafana dashboard
A "DomiNet — Energy" dashboard shows this balance in Grafana, with the **self-consumption** rate: see [observability](observability.md).

## Tests
`cargo test -p dominet-automation --test energy` (**13 days compared with frozen reference values `compute_daily_energy`**: daylight-saving nights, local-day edges, counter reset, day without data, `int(1.005 × 1000)` truncation), `-p dominet-domain --lib energy` (SVG without script or `style`, ticks), `-p dominet-server --test energy` (`seed-energy`, the 00:05 job) and `--lib` (`days` parameter), `-p dominet --test ui energy` (API, page), E2E `energy` (the real scheduler computes yesterday; page, ranges, API, live without reload).
