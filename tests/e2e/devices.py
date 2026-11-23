#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des appareils dans Firefox : administration (groupe, appareil avec liste deroulante), page /automation/ (cartes, filtres),
commandes envoyees au broker depuis la carte (boutons et curseur), etat publie sur le broker, appairage, page de l'appareil
(topics, image televersee puis supprimee, tableau Grafana). Le broker est celui de la stack de dev.

Usage : python3 tests/e2e/devices.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable), E2E_GRAFANA_DIR (dossier des tableaux Grafana, facultatif : si absent la
verification du fichier est omise). Le prefixe Zigbee2MQTT est remplace par un prefixe propre au script.
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
    sys.exit("usage: devices.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
grafana_dir = os.environ.get("E2E_GRAFANA_DIR")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
base = f"e2edev{int(time.time())}"
name = "E2E Lampe"

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def pub(topic, payload):
    body = payload if isinstance(payload, str) else json.dumps(payload)
    subprocess.run(["docker", "exec", broker, "mosquitto_pub", "-h", "localhost", "-q", "1", "-t", topic, "-m", body],
                   check=True, capture_output=True, stdin=subprocess.DEVNULL)


def listen(topic):
    """Ecoute un message sur le broker (en tache de fond) ; `heard(proc)` renvoie ce qui a ete recu."""
    return subprocess.Popen(["docker", "exec", broker, "mosquitto_sub", "-h", "localhost", "-q", "1", "-C", "1", "-W", "20", "-t", topic],
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, text=True)


def heard(proc):
    try:
        out, _ = proc.communicate(timeout=25)
    except subprocess.TimeoutExpired:
        proc.kill()
        return ""
    return out.strip()


def wait_until(predicate, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.3)
    return False


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def choose(selector, label_or_value):
    return m.js(
        f"const s = document.querySelector({selector!r}); if (!s) return false;"
        f"const o = [...s.options].find(o => o.value === {label_or_value!r} || o.textContent === {label_or_value!r}); if (!o) return false;"
        "s.value = o.value; s.dispatchEvent(new Event('change', {bubbles: true})); return true;"
    )


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def open_form():
    for _ in range(40):  # until the interface is hydrated, the click opens nothing
        m.click("#crud-add")
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


def save_enabled():
    return m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)


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
r.check("prefixe Zigbee2MQTT propre au script", res["status"] == 200 and "Saved" in res["text"], res["text"][:120])
time.sleep(1.5)

# --- administration: a group, then a device placed in that group ------------------------------------------------------------
m.goto(url + "admin/device-groups")
r.check("page des groupes", m.wait_for("document.querySelector('#crud-add') !== null", 10))
open_form()
save_enabled()
fill("#crud-field-name", "E2E Salon")
m.click("#crud-save")
r.check("groupe cree", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Salon')", 8))

m.goto(url + "admin/devices")
r.check("page des appareils (administration)", m.wait_for("document.querySelector('#crud-add') !== null", 10))
open_form()
save_enabled()
r.check("liste deroulante des groupes remplie depuis la base",
        m.wait_for("[...(document.querySelector('#crud-field-group_id')?.options || [])].some(o => o.textContent === 'E2E Salon')", 10))
fill("#crud-field-friendly_name", name)
choose("#crud-field-device_type", "light")
choose("#crud-field-group_id", "E2E Salon")
fill("#crud-field-base_topic", base)
fill("#crud-field-tracked_keys", "brightness")
m.click("#crud-save")
r.check("appareil cree", m.wait_for(f"[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === {name!r})", 8))
r.check("... avec son groupe et son type en base",
        scalar(f"SELECT d.device_type || '|' || g.name || '|' || d.base_topic || '|' || d.tracked_keys::text FROM device d JOIN device_group g ON g.id = d.group_id WHERE d.friendly_name = '{name}'")
        == f"light|E2E Salon|{base}|{{brightness}}")
dev = int(scalar(f"SELECT id FROM device WHERE friendly_name = '{name}'"))

# --- devices page: card, filters -------------------------------------------------------------------------------------------------
m.goto(url + "automation/")
r.check("la carte de l'appareil est affichee", m.wait_for(f"document.querySelector('#device-card-{dev}') !== null", 15))
r.check("temps reel connecte", m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20))
r.check("appareil sans etat : en attente", m.js(f"return document.querySelector('#device-card-{dev} #dev-waiting') !== null"))
m.goto(url + "automation/?type=sensor")
r.check("filtre par type : la lampe disparait", m.wait_for("document.querySelector('#devices-grid') !== null", 10) and m.js(f"return document.querySelector('#device-card-{dev}')") is None)
m.goto(url + "automation/?type=light")
r.check("filtre par type : la lampe est la", m.wait_for(f"document.querySelector('#device-card-{dev}') !== null", 10))
group = scalar("SELECT id FROM device_group WHERE name = 'E2E Salon'")
m.goto(url + f"automation/?group={group}")
r.check("filtre par groupe : la lampe est la", m.wait_for(f"document.querySelector('#device-card-{dev}') !== null", 10))
r.check("... et la liste deroulante garde le choix", m.js("return document.querySelector('#filter-group').value") == group)

# --- state published on the broker -> card ----------------------------------------------------------------------------------------
m.goto(url + "automation/")
m.wait_for(f"document.querySelector('#device-card-{dev}') !== null", 15)
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
m.js("window.__no_reload = 77")
pub(f"{base}/{name}", {"state": "OFF", "brightness": 100, "linkquality": 80})
r.check("l'etat publie arrive sur la carte (curseur de luminosite)",
        m.wait_for(f"document.querySelector('#device-card-{dev} #dev-brightness') !== null", 15), m.text(f"#device-card-{dev}"))
r.check("... sans recharger", m.js("return window.__no_reload") == 77)
ctrl_ready = f"document.querySelectorAll('#device-card-{dev} .vc-dev-ctrl').length === 2 && document.querySelectorAll('#device-card-{dev} .vc-dev-ctrl[disabled]').length === 0"
r.check("commandes actives", m.wait_for(ctrl_ready, 10))

# --- commands: the ON button and the slider publish on <device>/set ------------------------------------------------------------------
sub = listen(f"{base}/{name}/set")
time.sleep(1.0)
m.js(f"[...document.querySelectorAll('#device-card-{dev} .vc-dev-ctrl')].find(b => b.textContent.trim() === 'ON').click()")
got = heard(sub)
r.check("bouton ON : {\"state\":\"ON\"} publie sur le topic /set", got == '{"state":"ON"}', got)
sub = listen(f"{base}/{name}/set")
time.sleep(1.0)
m.js(
    f"const s = document.querySelector('#device-card-{dev} #dev-brightness'); s.value = '42';"
    "s.dispatchEvent(new Event('change', {bubbles: true}));"
)
got = heard(sub)
r.check("curseur de luminosite : {\"brightness\":42} publie", got == '{"brightness":42}', got)
pub(f"{base}/{name}/availability", {"state": "offline"})
r.check("appareil hors ligne : commandes inactives", m.wait_for(f"document.querySelectorAll('#device-card-{dev} .vc-dev-ctrl[disabled]').length === 2", 15))
pub(f"{base}/{name}/availability", "online")
r.check("de nouveau en ligne : commandes actives", m.wait_for(ctrl_ready, 15))

# --- pairing (administrator) -----------------------------------------------------------------------------------------------------
r.check("appairage ferme au depart", "fermé" in m.text("#permit-state"), m.text("#permit-state"))
sub = listen(f"{base}/bridge/request/permit_join")
time.sleep(1.0)
m.click("#permit-toggle")
got = heard(sub)
# Zigbee2MQTT 2.x format: a duration in seconds ({"value": true} is refused, "Invalid payload").
r.check("ouverture de l'appairage : {\"time\":254} publie", got == '{"time":254}', got)
r.check("... et l'etat affiche suit", m.wait_for("document.querySelector('#permit-state').textContent.includes('ouvert')", 8), m.text("#permit-state"))
m.wait_for("!document.querySelector('#permit-toggle').disabled", 8)  # the previous action is finished
m.click("#permit-toggle")
r.check("fermeture", m.wait_for("document.querySelector('#permit-state').textContent.includes('fermé')", 8))

# --- device page ----------------------------------------------------------------------------------------------------------------------
m.js(f"document.querySelector('#device-card-{dev} .vc-widget-header a').click()")
r.check("clic sur le nom : page de l'appareil", m.wait_for("location.pathname === '/automation/devices/%d' && document.querySelector('#device-title') !== null" % dev, 10), m.js("return location.href"))
info = m.text("#device-info")
r.check("topics affiches", f"{base}/{name}/set" in info and f"{base}/{name}" in info, info)

# Image: generated in the browser (canvas), sent by the form (multipart), viewed then deleted.
png = m.js_async(
    "const cb = arguments[arguments.length-1]; const c = document.createElement('canvas'); c.width = 800; c.height = 600;"
    "const g = c.getContext('2d'); g.fillStyle = '#2060c0'; g.fillRect(0, 0, 800, 600);"
    f"c.toBlob(b => {{ const f = new FormData(); f.append('image', b, 'x.png');"
    f"fetch('/api/devices/{dev}/image', {{method: 'POST', body: f, redirect: 'manual'}}).then(x => cb(x.type + ':' + x.status)); }}, 'image/png');"
)
r.check("televersement d'une image (redirection vers la page)", png in ("opaqueredirect:0", "basic:303", "opaqueredirect:303"), str(png))
m.goto(url + f"automation/devices/{dev}")
r.check("l'image est affichee sur la page", m.wait_for("document.querySelector('#device-live img.vc-device-img') !== null", 10))
size = m.js_async("const cb = arguments[arguments.length-1]; const i = document.querySelector('#device-live img.vc-device-img');"
                  "const w = () => cb([i.naturalWidth, i.naturalHeight].join('x')); if (i.complete) w(); else i.onload = w;")
r.check("... reduite a 400x300 au plus (800x600 envoyee)", size == "400x300", str(size))
m.goto(url + f"automation/devices/{dev}?image=unsupported")
r.check("message de refus lisible", m.wait_for("document.querySelector('#image-result') !== null", 10), m.text("#image-result"))
m.js_async(f"const cb = arguments[arguments.length-1]; fetch('/api/devices/{dev}/image/delete', {{method: 'POST', redirect: 'manual'}}).then(() => cb(1));")
m.goto(url + f"automation/devices/{dev}")
r.check("image supprimee : plus affichee", m.wait_for("document.querySelector('#device-admin') !== null", 10) and m.js("return document.querySelector('#device-live img.vc-device-img')") is None)

# History: a numeric value of the tracked key, then the curve.
pub(f"{base}/{name}", {"state": "ON", "brightness": 150})
time.sleep(0.5)
pub(f"{base}/{name}", {"state": "ON", "brightness": 200})
r.check("historique en base", wait_until(lambda: int(scalar(f"SELECT count(*) FROM device_state_log WHERE device_id = {dev} AND key = 'brightness'")) >= 1))
m.goto(url + f"automation/devices/{dev}")
r.check("courbe de la cle suivie", m.wait_for("document.querySelector('#device-history svg.vc-sparkline polyline') !== null", 10))

# Grafana dashboard.
m.wait_for("document.querySelector('#grafana-generate') && !document.querySelector('#grafana-generate').disabled", 10)
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)  # interface hydrated: the click has a handler
m.click("#grafana-generate")
r.check("tableau Grafana : ecrit", m.wait_for("(document.querySelector('#grafana-result')?.textContent || '').includes('écrit')", 10), m.text("#grafana-result"))
if grafana_dir:
    path = os.path.join(grafana_dir, "device_E2E_Lampe.json")
    ok = wait_until(lambda: os.path.exists(path), 5)
    doc = json.load(open(path)) if ok else {}
    r.check("... fichier JSON du tableau (un panneau par cle suivie)", ok and doc.get("uid") == f"dominet-device-{dev}" and len(doc.get("panels", [])) == 1, str(doc)[:200])
r.check("... et la case « tableau Grafana » est cochee", scalar(f"SELECT grafana_dashboard::text FROM device WHERE id = {dev}") == "true")

# An ordinary account does not see the administration tools.
r.check("les outils d'administration sont visibles de l'administrateur", m.js("return document.querySelector('#device-admin') !== null"))

# --- unknown device ----------------------------------------------------------------------------------------------------------------------
m.goto(url + f"automation/devices/{dev + 100000}")
r.check("appareil inconnu : message", m.wait_for("document.querySelector('#device-missing') !== null", 10))

r.finish()
