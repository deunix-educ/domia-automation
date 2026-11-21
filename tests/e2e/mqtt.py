#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E MQTT -> base -> temps reel -> widget, dans Firefox : le serveur ecoute le Mosquitto de dev ; on lui publie la decouverte
des appareils puis des etats ; un widget du tableau de bord change sans rechargement, passe en « indisponible » puis revient.

Usage : python3 tests/e2e/mqtt.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable, pose par tools/e2e.sh) ; le serveur doit avoir MQTT_HOST (pose par tools/e2e.sh).
Le prefixe Zigbee2MQTT est un reglage : on le remplace par un prefixe propre au script, pour ne rien melanger avec le broker de dev.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: mqtt.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
base = f"e2e{int(time.time())}/z2m"

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def pub(topic, payload, retain=False):
    body = payload if isinstance(payload, str) else json.dumps(payload)
    cmd = ["docker", "exec", broker, "mosquitto_pub", "-h", "localhost", "-q", "1", "-t", f"{base}/{topic}", "-m", body]
    if retain:
        cmd.append("-r")
    subprocess.run(cmd, check=True, capture_output=True, stdin=subprocess.DEVNULL)


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def one(query):
    return sql(query).strip().splitlines()


def scalar(query):
    out = sql(f"COPY ({query}) TO STDOUT;")
    return out.strip()


def wait_until(predicate, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.3)
    return False


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
login = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", login == 200)

# --- prefixe propre au script : la modification est prise en compte a chaud (abonnement refait) ----------------------------
res = post("/sf/settings/save", "key=Z2M_BASE_TOPIC&value=" + urllib.parse.quote(base, safe=""))
r.check("Zigbee2MQTT prefix changed", res["status"] == 200 and "Saved" in res["text"], res["text"][:120])
time.sleep(1.5)  # le client resouscrit

# --- decouverte : les appareils apparaissent en base, avec le type deduit ------------------------------------------------------
pub("bridge/devices", [
    {"ieee_address": "0x00124b0000000000", "friendly_name": "Coordinator", "type": "Coordinator", "definition": None},
    {"ieee_address": "0x00124b00000000aa", "friendly_name": "E2E Sonde", "type": "EndDevice", "supported": True,
     "definition": {"vendor": "Test", "model": "TH-01", "description": "Sonde", "exposes": [
         {"type": "numeric", "name": "temperature", "property": "temperature", "unit": "°C", "access": 1},
         {"type": "numeric", "name": "humidity", "property": "humidity", "unit": "%", "access": 1}]}},
])
r.check("appareil decouvert (type capteur deduit des exposes)",
        wait_until(lambda: scalar("SELECT device_type || '|' || vendor || '|' || base_topic FROM device WHERE friendly_name = 'E2E Sonde'") == f"sensor|Test|{base}"),
        scalar("SELECT device_type FROM device WHERE friendly_name = 'E2E Sonde'"))
r.check("le coordinateur est ignore", scalar("SELECT count(*) FROM device WHERE friendly_name = 'Coordinator'") == "0")
dev = int(scalar("SELECT id FROM device WHERE friendly_name = 'E2E Sonde'"))
sql("UPDATE device SET tracked_keys = ARRAY['temperature'] WHERE id = %d;" % dev)

# --- un tableau avec un widget sur cet appareil ------------------------------------------------------------------------------------
for d in json.loads(post("/sf/dashboards/list", "")["text"]):
    if d["owner"] == admin:
        post("/sf/dashboards/delete", f"id={d['id']}")
board = json.loads(post("/sf/dashboard/create", "name=MQTT")["text"])["Saved"]["id"]
sql(f"INSERT INTO widget (dashboard_id, widget_type, title, device_id, grid_col, grid_row, width, height) VALUES ({board}, 'device', 'Sonde', {dev}, 0, 0, 4, 3);")
m.goto(url + f"?d={board}")
r.check("widget affiche", m.wait_for("document.querySelector('[data-widget-type=device]') !== null", 20))
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
m.js("window.__no_reload = 5")

# --- etat publie sur le broker : widget mis a jour sans recharger ---------------------------------------------------------------------
pub("E2E Sonde", {"temperature": 23.4, "humidity": 51, "linkquality": 90})
r.check("l'etat publie sur MQTT arrive dans le widget (23.4)",
        m.wait_for("document.querySelector('[data-widget-type=device]').textContent.includes('23.4')", 15), m.text("[data-widget-type=device]"))
r.check("... sans rechargement", m.js("return window.__no_reload") == 5)
r.check("... et en base (etat remplace, disponible)", wait_until(lambda: scalar("SELECT available::text || '|' || (state->>'temperature') FROM device WHERE id = %d" % dev) == "true|23.4"))
pub("E2E Sonde", {"temperature": 24.1})
r.check("l'etat suivant REMPLACE le precedent (plus d'humidite)",
        m.wait_for("document.querySelector('[data-widget-type=device]').textContent.includes('24.1') && !document.querySelector('[data-widget-type=device]').textContent.includes('51')", 15))


def archived_state(device_id):
    """Dernier enregistrement de l'entree `dev_<id>` du bucket jetable de ReductStore : (etiquettes, corps JSON) ou None."""
    base_url, bucket = os.environ.get("REDUCTSTORE_URL"), os.environ.get("REDUCTSTORE_IOT_BUCKET")
    if not base_url or not bucket:
        return None
    req = urllib.request.Request(f"{base_url}/api/v1/b/{bucket}/dev_{device_id}",
                                 headers={"Authorization": "Bearer " + os.environ.get("REDUCTSTORE_TOKEN", "")})
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            labels = {k[len("x-reduct-label-"):]: v for k, v in resp.headers.items() if k.lower().startswith("x-reduct-label-")}
            return labels, json.loads(resp.read())
    except Exception:
        return None


if os.environ.get("REDUCTSTORE_URL"):
    r.check("archive brute ReductStore : etat complet et etiquette numerique de la cle suivie",
            wait_until(lambda: (a := archived_state(dev)) is not None and a[1].get("temperature") == 24.1 and a[0].get("temperature") == "24.1"),
            str(archived_state(dev)))
r.check("historique de la cle suivie", wait_until(lambda: int(scalar("SELECT count(*) FROM device_state_log WHERE device_id = %d AND key = 'temperature'" % dev)) >= 1))

# --- disponibilite ----------------------------------------------------------------------------------------------------------------
pub("E2E Sonde/availability", {"state": "offline"})
r.check("indisponible : cadre attenue", m.wait_for("document.querySelector('[data-widget-type=device]').classList.contains('vc-widget--offline')", 15))
pub("E2E Sonde/availability", "online")
r.check("de nouveau disponible", m.wait_for("!document.querySelector('[data-widget-type=device]').classList.contains('vc-widget--offline')", 15))

# --- bruit ignore -----------------------------------------------------------------------------------------------------------------------
before = scalar("SELECT state::text FROM device WHERE id = %d" % dev)
pub("E2E Sonde/set", {"state": "ON"})
pub("bridge/logs", {"level": "info"})
pub("E2E Sonde", "pas du json")
time.sleep(1.0)
r.check("commandes, journaux du bridge et charges non JSON ignores", scalar("SELECT state::text FROM device WHERE id = %d" % dev) == before)

r.check("le dernier etat est celui qui est en base", scalar("SELECT state->>'temperature' FROM device WHERE id = %d" % dev) == "24.1")

# nettoyage du prefixe pour ne pas laisser d'abonnement orphelin
post("/sf/settings/save", "key=Z2M_BASE_TOPIC&value=zigbee2mqtt")
r.finish()
