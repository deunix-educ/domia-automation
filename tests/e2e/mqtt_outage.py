#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la reprise apres coupure du broker, dans Firefox : on ARRETE le Mosquitto de la stack de dev quelques secondes, puis on le
relance. Pendant la coupure, une commande est refusee tout de suite avec un message (pas de file d'attente) ; apres, SANS redemarrer le
serveur, les etats publies arrivent de nouveau (abonnements repris) et les commandes repartent.

Usage : python3 tests/e2e/mqtt_outage.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL ; E2E_MQTT_CONTAINER (conteneur du broker de dev). Le broker est TOUJOURS relance a la fin, meme en cas d'echec.
"""
import json
import os
import subprocess
import sys
import time
import urllib.parse

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: mqtt_outage.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
base = f"e2eout{int(time.time())}"
name = "E2E Coupure"

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def docker(*args):
    return subprocess.run(["docker", *args], check=True, capture_output=True, stdin=subprocess.DEVNULL)


def pub(topic, payload):
    body = payload if isinstance(payload, str) else json.dumps(payload)
    docker("exec", broker, "mosquitto_pub", "-h", "localhost", "-q", "1", "-t", topic, "-m", body)


def wait_until(predicate, seconds=30, step=0.4):
    end = time.time() + seconds
    while time.time() < end:
        try:
            if predicate():
                return True
        except subprocess.CalledProcessError:
            pass
        time.sleep(step)
    return False


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


try:
    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    m.js("localStorage.clear()")
    res = m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )
    r.check("connexion administrateur", res == 200, str(res))
    res = post("/sf/settings/save", "key=Z2M_BASE_TOPIC&value=" + urllib.parse.quote(base, safe=""))
    r.check("prefixe Zigbee2MQTT propre au script", res["status"] == 200 and "Saved" in res["text"])
    time.sleep(1.5)
    sql(f"INSERT INTO device (friendly_name, device_type, base_topic) VALUES ('{name}', 'switch', '{base}');")
    dev = int(scalar(f"SELECT id FROM device WHERE friendly_name = '{name}'"))

    m.goto(url + "automation/")
    m.wait_for(f"document.querySelector('#device-card-{dev}') !== null", 15)
    m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
    pub(f"{base}/{name}", {"state": "OFF"})
    on = f"[...document.querySelectorAll('#device-card-{dev} .vc-dev-ctrl')].find(b => b.textContent.trim() === 'ON')"
    r.check("avant la coupure : l'etat arrive et la commande est possible",
            m.wait_for(f"(() => {{ const b = {on}; return !!b && !b.disabled; }})()", 15), m.text(f"#device-card-{dev}"))

    # --- coupure du broker ---------------------------------------------------------------------------------------------------------------
    docker("stop", "-t", "1", broker)
    time.sleep(2.0)
    m.js(f"{on}.click()")
    r.check("pendant la coupure : commande refusee tout de suite, avec un message",
            m.wait_for(f"(document.querySelector('#dev-msg-{dev}')?.textContent || '').includes('injoignable')", 10),
            m.text(f"#dev-msg-{dev}"))
    r.check("le serveur reste joignable pendant la coupure", m.js_async(
        "const cb = arguments[arguments.length-1]; fetch('/healthz').then(x => cb(x.status));") == 200)

    # --- retour du broker : reprise sans redemarrer le serveur --------------------------------------------------------------------------
    docker("start", broker)
    ok = wait_until(lambda: (pub(f"{base}/{name}", {"state": "ON", "marqueur": int(time.time())}) or True)
                    and scalar(f"SELECT state->>'state' FROM device WHERE id = {dev}") == "ON", seconds=40, step=1.0)
    r.check("apres le retour du broker : les etats publies arrivent de nouveau en base (abonnements repris)", ok,
            scalar(f"SELECT state::text FROM device WHERE id = {dev}"))
    r.check("... et sur la carte, sans recharger la page",
            m.wait_for(f"document.querySelector('#device-card-{dev}')?.textContent.includes('Marqueur')", 15), m.text(f"#device-card-{dev}"))

    r.check("le message de refus de la coupure s'efface quand l'appareil republie un etat",
            m.wait_for(f"!(document.querySelector('#dev-msg-{dev}')?.textContent || '').includes('injoignable')", 10), m.text(f"#dev-msg-{dev}"))

    sub = subprocess.Popen(["docker", "exec", broker, "mosquitto_sub", "-h", "localhost", "-q", "1", "-C", "1", "-W", "20", "-t", f"{base}/{name}/set"],
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, text=True)
    time.sleep(1.0)
    m.js(f"{on}.click()")
    try:
        heard, _ = sub.communicate(timeout=25)
    except subprocess.TimeoutExpired:
        sub.kill()
        heard = ""
    r.check("apres la coupure : la commande repart (publiee sur le topic /set)", heard.strip() == '{"state":"ON"}', heard.strip())
finally:
    subprocess.run(["docker", "start", broker], capture_output=True, stdin=subprocess.DEVNULL)

r.finish()
