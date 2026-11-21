# 0036 — Daily energy balance

- Status: accepted
- Date: 2026-11-03

## Context
The reference implementation aggregates every night (00:05) the counter history into a per-day balance (`EnergyDailyLog`): Linky peak and off-peak, solar production, heat pump. A page shows the last 30 balances (a Chart.js chart loaded from a CDN), the live state, and a JSON API. The requirement: the same computation, verified against the reference, a chart without a JavaScript library, and the means to put devices in a state to produce the history.

## Decisions
- **Same computation as `compute_daily_energy`**, run by the reference implementation as the oracle:
  - the day is the **local day** (from local midnight to the next local midnight: 23 h on the day of the spring change, 25 h on the day of the autumn change) — adding one day to a zoned datetime is a wall-clock addition;
  - cumulative counters (`HCHP`, `HCHC`, `energy_a`, `energy_b`): maximum − minimum of the day, **truncated towards zero**, never negative; text values are ignored;
  - production = maximum of `qdy` (kWh, reset every day) × 1000, truncated (`int(1.005 * 1000)` is 1004 in the reference: same float, same result);
  - a missing device counts zero (warning).
  13 days (including daylight-saving nights, the last and first second of a local day, a value reset to zero, a day without data) are identical. Expected values are frozen in `tests/fixtures/energy/`.
- **Devices**: `ENERGY_LINKY_DEVICE`, `ENERGY_SOLAR_DEVICE`, `ENERGY_PAC_DEVICE` (defaults `consommation`, `prod`, `PAC-CE`), read from the configuration. The computation lives in `dominet_automation::energy`, generic over the repositories.
- **Who runs it**: the system job `compute_daily_energy` of the scheduler (00:05 local, ADR 0033), which computes **yesterday** (local day) or `args.date`; `dominet aggregate-energy [--days N | --date YYYY-MM-DD]` to catch up. Recomputing replaces.
- **`dominet seed-energy`** adds (without removing anything) to the three devices' tracked keys those the computation needs, and the live powers shown by the Grafana energy dashboard: without history of these keys the balance stays at zero and nothing says so. A missing device is reported, not created.
- **Deliberate deviation: the last `N` days.** The reference returned the first 30 recorded balances: the page froze after thirty days. `EnergyRepo::recent(n)` returns the last n, oldest to newest.
- **API** `GET /api/energy/?days=N` (signed-in account; 30 by default, 365 at most, unreadable = 30) as `{"data": [...]}` with computed `linky_total_wh` and `net_wh`.
- **Page** `/automation/energy`: four live cards (solar production `pac` and `qdy` of the day, Linky consumption `PAPP` and `PTEC`, heat pump `power_ab`, net balance = production − consumption − heat pump with "surplus" / "deficit") fed by the `device:<id>` topics; 7 / 14 / 30 / 90 day ranges; a table, most recent first. Today's production is shown in **kWh**.
- **Server-computed SVG chart** (`dominet_domain::energy::chart_svg`, tested): solar / Linky / heat pump bars and a net-balance curve, "round" ticks (1, 2, 5 × 10ⁿ), a negative axis when a day is in deficit, tooltips (`<title>`). No script, no `style` attribute (CSP): inserted through `inner_html` of a text fully produced by the server from numbers and dates.

## Consequences
- Without key tracking (`seed-energy`) or history, the page says so and offers the two commands.
- The current day's balance only exists after 00:05: the page shows "today" only live.
- Any day can be recomputed with `aggregate-energy`.
