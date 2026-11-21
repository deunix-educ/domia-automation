#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des publishers dans Firefox, contre l'API Open-Meteo et l'onduleur SolarMax SIMULES (lances par tools/e2e.sh) : une station creee
depuis l'administration est interrogee tout de suite, son releve publie sur le broker de dev devient un appareil (source MQTT) visible
sur /automation/ ; l'onduleur simule alimente un second appareil ; desactiver la station arrete les requetes.

Usage : python3 tests/e2e/weather.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL, E2E_METEO_COUNT_URL (compteur de requetes du simulateur meteo), E2E_SOLARMAX_TOPIC (topic de l'onduleur simule).
"""
import json
import os
import sys
import time
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: weather.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
count_url, sun_topic = os.environ.get("E2E_METEO_COUNT_URL"), os.environ.get("E2E_SOLARMAX_TOPIC")
db_shell = os.environ.get("E2E_DB_SHELL")
if not (db_shell and count_url and sun_topic):
    sys.exit("E2E_DB_SHELL, E2E_METEO_COUNT_URL ou E2E_SOLARMAX_TOPIC absent : lancer par tools/e2e.sh")
suffix = f"e2emeteo{int(time.time())}"
sun_prefix = sun_topic.split("/")[0]

m = Marionette()
r = Report()


def scalar(query):
    import subprocess
    out = subprocess.run(db_shell, shell=True, input=f"COPY ({query}) TO STDOUT;".encode(), check=True, capture_output=True)
    return out.stdout.decode().strip()


def requests_made():
    with urllib.request.urlopen(count_url, timeout=5) as resp:
        return json.loads(resp.read())["count"]


def wait_until(predicate, seconds=20):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.3)
    return False


def post_json(path, payload):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: {json.dumps(json.dumps(payload))}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def source(name, pattern, base):
    return {"input": {"id": None, "name": name, "topic_pattern": pattern, "qos": 1, "active": True, "log_messages": False,
                      "auto_device": True, "auto_device_type": "sensor", "base_topic": base, "handler_code": "", "description": ""}}


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# Les sources qui font des messages des appareils (comme `dominet seed-weather` et la source SolarMax de la production).
for payload in (source("E2E Météo", "meteo/#", "meteo"), source("E2E Soleil", f"{sun_prefix}/#", sun_prefix)):
    res = post_json("/sf/mqtt-sources/save", payload)
    r.check(f"source {payload['input']['name']}", res["status"] == 200 and "Saved" in res["text"], res["text"][:100])
time.sleep(1.5)  # abonnements recharges

# --- l'onduleur simule alimente un appareil (publisher SolarMax, toutes les 2 s) ---------------------------------------------------------
r.check("onduleur simule -> appareil (production, etat)",
        wait_until(lambda: scalar("SELECT (state->>'pac') || '|' || (state->>'status') FROM device WHERE friendly_name = 'inv/prod'") == "632.5|Operating at MPP"),
        scalar("SELECT state::text FROM device WHERE friendly_name = 'inv/prod'"))
r.check("... avec le prefixe de la source", scalar("SELECT base_topic FROM device WHERE friendly_name = 'inv/prod'") == sun_prefix)

# --- station meteo creee depuis l'administration : interrogee tout de suite ----------------------------------------------------------------
before = requests_made()
r.check("sans station active, l'API n'est pas interrogee", wait_until(lambda: requests_made() == before, 3) and scalar("SELECT count(*) FROM weather_station") == "0")
m.goto(url + "admin/weather-stations")
r.check("page des stations", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("entree du menu d'administration", m.js("return !!document.querySelector('#vc-nav-admin a[href=\"/admin/weather-stations\"]')") is True)
for _ in range(40):
    m.click("#crud-add")
    if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
        break
m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)
fill("#crud-field-name", "E2E Maison")
fill("#crud-field-latitude", "91")
fill("#crud-field-longitude", "5,4")
fill("#crud-field-mqtt_topic_suffix", suffix)
fill("#crud-field-refresh_minutes", "1")
m.click("#crud-save")
r.check("latitude hors plage : erreur sous le champ",
        m.wait_for("(document.querySelector('#crud-error-latitude')?.textContent || '').length > 0", 8), m.text("#crud-error-latitude"))
fill("#crud-field-latitude", "43,3")
m.click("#crud-save")
r.check("station creee (virgule decimale acceptee)",
        m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Maison')", 8))
r.check("... enregistree en base", scalar(f"SELECT latitude || '|' || longitude || '|' || enabled FROM weather_station WHERE mqtt_topic_suffix = '{suffix}'") == "43.3|5.4|true")
r.check("le publisher interroge l'API des la creation (sans attendre l'intervalle)", wait_until(lambda: requests_made() > before, 10), str(requests_made()))
r.check("le releve publie devient un appareil (source MQTT)",
        wait_until(lambda: scalar(f"SELECT (state->>'temperature') || '|' || base_topic FROM device WHERE friendly_name = '{suffix}'") == "18.2|meteo"),
        scalar(f"SELECT state::text FROM device WHERE friendly_name = '{suffix}'"))

# --- visible sur la page des appareils ----------------------------------------------------------------------------------------------------------
m.goto(url + "automation/")
r.check("la station meteo est sur /automation/ avec sa temperature",
        m.wait_for(f"[...document.querySelectorAll('#devices-grid article')].some(a => a.textContent.includes({suffix!r}) && a.textContent.includes('18.2'))", 15),
        m.text("#devices-grid"))
r.check("l'onduleur aussi (puissance)", m.wait_for("[...document.querySelectorAll('#devices-grid article')].some(a => a.textContent.includes('inv/prod') && a.textContent.includes('632.5'))", 15))

# --- desactiver la station arrete les requetes ------------------------------------------------------------------------------------------------------
m.goto(url + "admin/weather-stations")
m.wait_for("document.querySelector('#crud-add') !== null", 10)
m.wait_for(f"[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Maison')", 10)
for _ in range(40):  # tant que l'interface n'est pas hydratee, le clic n'ouvre rien
    m.js("[...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === 'E2E Maison').querySelector('.crud-edit').click()")
    if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
        break
m.wait_for("document.querySelector('#crud-field-enabled') !== null && document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 10)
m.js("document.querySelector('#crud-field-enabled').click()")
m.click("#crud-save")
r.check("station desactivee en base", wait_until(lambda: scalar(f"SELECT enabled::text FROM weather_station WHERE mqtt_topic_suffix = '{suffix}'") == "false"))
time.sleep(1.5)  # le reveil du publisher a pu faire un dernier cycle (sans station active : aucune requete)
settled = requests_made()
time.sleep(4)
r.check("plus aucune requete a l'API apres la desactivation", requests_made() == settled, f"{settled} -> {requests_made()}")

r.finish()
