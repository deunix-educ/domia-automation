#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de l'energie dans Firefox (R4 lot 7) : un historique de compteurs sur les trois appareils du bilan, le travail de 00:05
(`compute_daily_energy`) lance par le VRAI planificateur (son prochain passage est avance a la main), la page /automation/energy
(graphique SVG, tableau, plages), l'API /api/energy/ et l'etat en direct (production, Linky, PAC, bilan net) sans recharger.

Usage : python3 tests/e2e/energy.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable).
"""
import datetime as dt
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: energy.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def wait_until(predicate, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.4)
    return False


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# --- history: yesterday, on the summary's three devices (local time = Paris, noon UTC stays the same day) ------------------------------
sql("INSERT INTO device (friendly_name, base_topic, tracked_keys) VALUES ('consommation', 'x', '{HCHP,HCHC}'), ('prod', 'x', '{qdy}'), ('PAC-CE', 'x', '{energy_a,energy_b}');")
yesterday_local = scalar("SELECT (now() AT TIME ZONE 'Europe/Paris')::date - 1")
rows = []
for dev, key, values in [
    ("consommation", "HCHP", [100000.0, 101200.9]),
    ("consommation", "HCHC", [50000.0, 50300.0]),
    ("PAC-CE", "energy_a", [2000.0, 2400.0]),
    ("PAC-CE", "energy_b", [1000.0, 1100.5]),
    ("prod", "qdy", [0.5, 6.005]),
]:
    for hour, v in zip((6, 14), values):  # 06:00 and 14:00 UTC: same local day as UTC
        rows.append(f"((SELECT id FROM device WHERE friendly_name = '{dev}'), '{key}', '{yesterday_local} {hour:02d}:00:00+00', {v})")
sql("INSERT INTO device_state_log (device_id, key, ts, value_num) VALUES " + ",".join(rows) + ";")
# Fourteen days of older summaries, for the chart.
sql(f"""INSERT INTO energy_daily_log (day, linky_hp_wh, linky_hc_wh, solar_wh, pac_wh)
        SELECT '{yesterday_local}'::date - g, 3000 + g * 10, 800, 5000 + g * 100, 400 FROM generate_series(1, 14) g;""")

# --- the 00:05 job, launched by the real scheduler ----------------------------------------------------------------------------------------
r.check("le travail systeme existe (00:05 locale)", scalar("SELECT cron_expr || '|' || timezone FROM schedule_job WHERE name = 'compute_daily_energy'") == "5 0 * * *|Europe/Paris")
sql("UPDATE schedule_job SET args = '{}'::jsonb, next_run_at = now() - interval '1 second' WHERE name = 'compute_daily_energy';")
r.check("hier calcule par le planificateur", wait_until(lambda: (scalar("SELECT last_status FROM schedule_job WHERE name = 'compute_daily_energy'") or "").startswith(f"ok : balance of {yesterday_local}"), 45),
        scalar("SELECT last_status FROM schedule_job WHERE name = 'compute_daily_energy'"))
got = scalar(f"SELECT linky_hp_wh || '|' || linky_hc_wh || '|' || solar_wh || '|' || pac_wh FROM energy_daily_log WHERE day = '{yesterday_local}'")
r.check("... les memes Wh que la reference (HP 1200, HC 300, solaire 6005 = int(6.005*1000), PAC 500)", got == "1200|300|6005|500", got)

# --- the page --------------------------------------------------------------------------------------------------------------------------------
m.goto(url + "automation/energy")
r.check("page d'energie", m.wait_for("document.querySelector('#energy-live') !== null", 15))
r.check("graphique SVG calcule par le serveur (15 jours x 3 barres)", m.wait_for("document.querySelectorAll('#energy-chart rect').length === 45", 10), str(m.js("return document.querySelectorAll('#energy-chart rect').length")))
r.check("... sans script ni attribut style", m.js("return document.querySelector('#energy-chart').querySelectorAll('script, [style]').length") == 0)
r.check("tableau : le plus recent d'abord, hier en tete", m.js("return document.querySelector('#energy-table tbody tr').dataset.date") == yesterday_local)
r.check("... bilan net d'hier = 6005 - 1500 - 500 = +4005", "+4005" in (m.js("return document.querySelector('#energy-table tbody tr').textContent") or ""))
m.click("#energy-range-7")
r.check("plage 7 jours", m.wait_for("document.querySelectorAll('#energy-table tbody tr').length === 7", 10), str(m.js("return document.querySelectorAll('#energy-table tbody tr').length")))
r.check("... et 7 jours de barres", m.js("return document.querySelectorAll('#energy-chart rect').length") == 21)
api = json.loads(m.js_async("const cb = arguments[arguments.length-1]; fetch('/api/energy/?days=5').then(async x => cb(JSON.stringify({status: x.status, body: await x.json()}))).catch(() => cb(JSON.stringify({status: 0})));"))
r.check("API /api/energy/?days=5 : les cinq derniers jours, format de reference",
        api["status"] == 200 and len(api["body"]["data"]) == 5 and api["body"]["data"][-1]["date"] == yesterday_local
        and api["body"]["data"][-1]["linky_total_wh"] == 1500 and api["body"]["data"][-1]["net_wh"] == 4005, str(api)[:200])

# --- live state, without reloading -----------------------------------------------------------------------------------------------------------
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
m.js("window.__no_reload = 5")
ids = {n: scalar(f"SELECT id FROM device WHERE friendly_name = '{n}'") for n in ("prod", "consommation", "PAC-CE")}
post("/sf/devices/inject", f"device_id={ids['prod']}&state=" + json.dumps({"pac": 1500, "qdy": 7.5}).replace('"', "%22").replace(" ", "") + "&available=true")
post("/sf/devices/inject", f"device_id={ids['consommation']}&state=" + json.dumps({"PAPP": 400, "PTEC": "HP"}).replace('"', "%22").replace(" ", "") + "&available=true")
post("/sf/devices/inject", f"device_id={ids['PAC-CE']}&state=" + json.dumps({"power_ab": 300}).replace('"', "%22").replace(" ", "") + "&available=true")
r.check("production solaire en direct", m.wait_for("document.querySelector('#energy-live-solar').textContent.trim() === '1500'", 15), m.text("#energy-live-solar"))
r.check("... production du jour en kWh", "7.5" in (m.text("#energy-live-qdy") or ""), m.text("#energy-live-qdy"))
r.check("consommation Linky et tarif en direct", m.wait_for("document.querySelector('#energy-live-papp').textContent.trim() === '400' && document.querySelector('#energy-live-ptec').textContent.includes('HP')", 15))
r.check("PAC en direct", m.wait_for("document.querySelector('#energy-live-pac').textContent.trim() === '300'", 15))
r.check("bilan net en direct = 1500 - 400 - 300 = +800 (surplus)", m.wait_for("document.querySelector('#energy-live-net').textContent.trim() === '+800'", 15) and "Surplus" in (m.text("#energy-live-net-label") or ""), m.text("#energy-live-net"))
r.check("... sans recharger la page", m.js("return window.__no_reload") == 5)

# Cleanup: the throw-away database is shared with the following scripts.
sql("DELETE FROM energy_daily_log; DELETE FROM device WHERE friendly_name IN ('consommation', 'prod', 'PAC-CE');")
sys.exit(r.finish())
