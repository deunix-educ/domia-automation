#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des alertes dans Firefox (R4 lot 6), sur le broker de dev : regle creee depuis l'administration (champ invalide refuse), etat
publie -> alerte, bandeau et pastille de la barre laterale en direct, page /alerts (filtres, acquittement, tout acquitter), la
particularite de la reference (une alerte acquittee sort du cooldown : l'etat chaud suivant en recree une), resolution au retour a la
normale.

Usage : python3 tests/e2e/alerts.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable). Le prefixe Zigbee2MQTT est remplace par un prefixe propre au script.
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
    sys.exit("usage: alerts.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
z2m = f"e2ealert{int(time.time())}"
dev = "E2E Meteo"

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def pub(payload):
    subprocess.run(["docker", "exec", broker, "mosquitto_pub", "-h", "localhost", "-q", "1", "-t", f"{z2m}/{dev}", "-m", json.dumps(payload)],
                   check=True, capture_output=True, stdin=subprocess.DEVNULL)


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
    for _ in range(40):
        m.click("#crud-add")
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


def badge():
    """Texte de la pastille (None si elle est cachee)."""
    return m.js("const b = document.querySelector('#vc-alert-badge'); return b && !b.hidden ? b.textContent.trim() : null")


def rows(status=None):
    return m.js("return [...document.querySelectorAll('#alerts-table tbody tr')].map(t => t.dataset.status).join(',')")


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
res = post("/sf/settings/save", "key=Z2M_BASE_TOPIC&value=" + urllib.parse.quote(z2m, safe=""))
r.check("prefixe Zigbee2MQTT propre au script", res["status"] == 200 and "Saved" in res["text"], res["text"][:120])
sql(f"INSERT INTO device (friendly_name, base_topic) VALUES ('{dev}', '{z2m}');")
time.sleep(1.5)

# --- administration: a rule -------------------------------------------------------------------------------------------------------------
m.goto(url + "admin/alert-rules")
r.check("page des regles d'alerte", m.wait_for("document.querySelector('#crud-add') !== null", 10))
open_form()
m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)
r.check("liste des appareils remplie depuis la base", m.wait_for(f"[...(document.querySelector('#crud-field-device_id')?.options || [])].some(o => o.textContent === {dev!r})", 10))
fill("#crud-field-name", "E2E Canicule")
choose("#crud-field-device_id", dev)
fill("#crud-field-key", "temperature")
fill("#crud-field-threshold_num", "abc")
m.click("#crud-save")
r.check("seuil illisible : erreur sur le champ", m.wait_for("document.querySelector('#crud-error-threshold_num') !== null", 10), m.text("#crud-error-threshold_num"))
fill("#crud-field-threshold_num", "35,5")
fill("#crud-field-cooldown_minutes", "60")
m.click("#crud-save")
r.check("regle creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Canicule')", 10))
r.check("... seuil a virgule lu : 35.5 en base", scalar("SELECT threshold_num FROM alert_rule WHERE name = 'E2E Canicule'") == "35.5")
time.sleep(1.5)  # the evaluation re-reads the rules

# --- hot state -> alert, banner, badge ------------------------------------------------------------------------------------------------------
m.goto(url + "alerts")
r.check("page des alertes", m.wait_for("document.querySelector('#alerts-filters') !== null", 10))
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
r.check("aucune alerte au depart : pastille cachee", badge() is None)
pub({"temperature": 22.0})
time.sleep(1.2)
pub({"temperature": 40.0})
r.check("alerte declenchee en base", wait_until(lambda: scalar("SELECT count(*) FROM alert WHERE status = 'firing'") == "1"), scalar("SELECT count(*) FROM alert"))
smtp_file = os.environ.get("E2E_SMTP_FILE")
if smtp_file:
    def sent_mails():
        with open(smtp_file) as f:
            return [json.loads(x) for x in f if x.strip()]
    r.check("e-mail d'alerte envoye (SMTP simule)", wait_until(lambda: any("E2E Canicule" in x["data"] for x in sent_mails())), str(sent_mails())[:200])
    mail = next((x for x in sent_mails() if "E2E Canicule" in x["data"]), {"data": "", "to": []})
    r.check("... subject « [DomiNet Alert ... ] rule », texte long", "Subject: [DomiNet Alert " in mail["data"] and "Device: E2E Meteo" in mail["data"], mail["data"][:200])
    r.check("... alerte marquee notifiee en base", wait_until(lambda: scalar("SELECT bool_and(notified) FROM alert") == "t"), scalar("SELECT notified FROM alert"))
r.check("bandeau dans le navigateur", m.wait_for("[...document.querySelectorAll('.vc-toast')].some(t => t.textContent.includes('E2E Canicule') && t.textContent.includes('40'))", 15))
r.check("pastille de la barre laterale = 1, sans recharger", m.wait_for("(() => { const b = document.querySelector('#vc-alert-badge'); return b && !b.hidden && b.textContent.trim() === '1'; })()", 15), str(badge()))
r.check("la liste se met a jour toute seule", m.wait_for("document.querySelectorAll('#alerts-table tbody tr[data-status=firing]').length === 1", 15), rows())
r.check("... avec la regle, l'appareil et la valeur",
        m.js("const t = document.querySelector('#alerts-table tbody tr').textContent; return t.includes('E2E Canicule') && t.includes('E2E Meteo') && t.includes('40')"))
pub({"temperature": 41.0})
time.sleep(1.5)
r.check("cooldown de 60 min : pas de seconde alerte", scalar("SELECT count(*) FROM alert") == "1")

# --- acknowledgement: the alert leaves the cooldown (a quirk of the reference) -------------------------------------------------------------------
m.wait_for("document.querySelector('.alert-ack') && !document.querySelector('.alert-ack').disabled", 15)
m.click(".alert-ack")
r.check("acquittee : la pastille disparait", m.wait_for("(() => { const b = document.querySelector('#vc-alert-badge'); return !b || b.hidden; })()", 15), str(badge()))
r.check("... acquittee par ce compte en base", scalar(f"SELECT status || '|' || (SELECT username FROM users WHERE id = acknowledged_by) FROM alert") == f"acknowledged|{admin}")
pub({"temperature": 42.0})
r.check("etat chaud suivant : nouvelle alerte aussitot (reference)", wait_until(lambda: scalar("SELECT count(*) FROM alert") == "2"), scalar("SELECT count(*) FROM alert"))
r.check("... pastille = 1", m.wait_for("(() => { const b = document.querySelector('#vc-alert-badge'); return b && !b.hidden && b.textContent.trim() === '1'; })()", 15))

# --- return to normal: resolved ------------------------------------------------------------------------------------------------------------------
pub({"temperature": 20.0})
r.check("retour a la normale : resolue", wait_until(lambda: scalar("SELECT count(*) FROM alert WHERE status = 'resolved'") == "1"), scalar("SELECT string_agg(status, ',') FROM alert"))
r.check("... pastille cachee", m.wait_for("(() => { const b = document.querySelector('#vc-alert-badge'); return !b || b.hidden; })()", 15))

# --- filters ----------------------------------------------------------------------------------------------------------------------------------------------
m.click("#alerts-filter-all")
r.check("filtre « toutes » : deux alertes", m.wait_for("document.querySelectorAll('#alerts-table tbody tr').length === 2", 10), rows())
m.click("#alerts-filter-resolved")
r.check("filtre « resolues » : une alerte", m.wait_for("document.querySelectorAll('#alerts-table tbody tr[data-status=resolved]').length === 1", 10), rows())
m.click("#alerts-filter-acknowledged")
r.check("filtre « acquittees » : une alerte, avec qui l'a acquittee",
        m.wait_for(f"document.querySelectorAll('#alerts-table tbody tr[data-status=acknowledged]').length === 1 && document.querySelector('#alerts-table tbody tr').textContent.includes({admin!r})", 10), rows())
m.click("#alerts-filter-firing")
r.check("filtre « en cours » : vide", m.wait_for("document.querySelector('#alerts-empty') !== null", 10))

# --- acknowledge all -------------------------------------------------------------------------------------------------------------------------------------------
pub({"temperature": 43.0})
r.check("nouvelle alerte apres la resolution", wait_until(lambda: scalar("SELECT count(*) FROM alert WHERE status = 'firing'") == "1"))
m.wait_for("document.querySelectorAll('#alerts-table tbody tr[data-status=firing]').length === 1", 15)
m.wait_for("!document.querySelector('#alerts-ack-all').disabled", 10)
m.click("#alerts-ack-all")
r.check("tout acquitter : plus rien en cours", wait_until(lambda: scalar("SELECT count(*) FROM alert WHERE status = 'firing'") == "0"))
r.check("... la pastille disparait", m.wait_for("(() => { const b = document.querySelector('#vc-alert-badge'); return !b || b.hidden; })()", 15))

# --- rule deletion: the alerts stay ------------------------------------------------------------------------------------------------------------------
m.goto(url + "admin/alert-rules")
m.wait_for("document.querySelector('#crud-add') !== null", 10)
m.wait_for("document.querySelectorAll('#crud-table tbody tr').length > 0", 10)
for _ in range(40):
    m.js("document.querySelector('#crud-table tbody tr .crud-delete')?.click()")
    if m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 0.5):
        break
m.click(".crud-confirm-yes")
r.check("regle supprimee", m.wait_for("document.querySelectorAll('#crud-table tbody tr').length === 0", 10))
r.check("... les alertes deja creees restent, sans regle", scalar("SELECT count(*) FROM alert WHERE rule_id IS NULL") == "3", scalar("SELECT count(*) FROM alert"))

# Cleanup: the throw-away database is shared with the following scripts.
sql(f"DELETE FROM device WHERE friendly_name = '{dev}';")
sys.exit(r.finish())
