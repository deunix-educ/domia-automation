#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des sources MQTT dans Firefox : on ajoute une source depuis la page /admin/mqtt-sources (appareil automatique), on publie
sur le Mosquitto de dev, l'appareil apparait ; on desactive la source depuis la page, les messages suivants sont ignores ;
on la supprime. Les abonnements sont rechargés à chaud, sans redemarrage.

Usage : python3 tests/e2e/mqtt_sources.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable, pose par tools/e2e.sh) ; le serveur doit avoir MQTT_HOST.
"""
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: mqtt_sources.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
prefix = f"e2esrc{int(time.time())}"
name = "E2E Compteur"

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


def rows():
    return m.js("return [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent)") or []


def save_enabled():
    return m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)


def click_in_row(label, selector):
    return m.js(
        "const tr = [...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === "
        f"{label!r}); if (!tr) return false; tr.querySelector({selector!r}).click(); return true;"
    )


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
m.goto(url + "admin/mqtt-sources")
r.check("page rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("entree du menu d'administration",
        m.js("return !!document.querySelector('#vc-nav-admin a[href=\"/admin/mqtt-sources\"]')") is True)

# --- ajout depuis la page ----------------------------------------------------------------------------------------------------
for _ in range(40):  # tant que l'interface n'est pas hydratee, le clic n'ouvre rien
    m.click("#crud-add")
    if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
        break
save_enabled()
fill("#crud-field-name", name)
fill("#crud-field-topic_pattern", "a/#/b")  # invalide : # au milieu
m.click("#crud-save")
r.check("filtre invalide : erreur sous le champ",
        m.wait_for("(document.querySelector('#crud-error-topic_pattern')?.textContent || '').length > 0", 8),
        m.text("#crud-error-topic_pattern"))
fill("#crud-field-topic_pattern", f"{prefix}/#")
fill("#crud-field-base_topic", prefix)
m.js("document.querySelector('#crud-field-auto_device').click()")
m.js("const s = document.querySelector('#crud-field-qos'); s.value = '1'; s.dispatchEvent(new Event('change', {bubbles: true}));")
fill("#crud-field-handler_code", 'print("conserve")')
m.click("#crud-save")
r.check("la source est dans le tableau", m.wait_for(f"[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === {name!r})", 8), str(rows()))
r.check("enregistree en base (QoS 1, appareil automatique, code conserve)",
        scalar(f"SELECT qos || '|' || auto_device || '|' || handler_code FROM mqtt_source WHERE name = '{name}'") == '1|true|print("conserve")')
time.sleep(1.5)  # abonnement recharge

# --- un message sur le broker cree l'appareil -----------------------------------------------------------------------------------
pub(f"{prefix}/compteur/prod", {"power": 1500})
r.check("appareil cree par la source (nom sans prefixe, prefixe memorise)",
        wait_until(lambda: scalar("SELECT friendly_name || '|' || base_topic || '|' || (state->>'power') FROM device WHERE friendly_name = 'compteur/prod'") == f"compteur/prod|{prefix}|1500"),
        scalar("SELECT count(*) FROM device"))
pub(f"{prefix}/compteur/prod", {"power": 1600})
r.check("le message suivant remplace l'etat", wait_until(lambda: scalar("SELECT state->>'power' FROM device WHERE friendly_name = 'compteur/prod'") == "1600"))
pub(f"{prefix}/compteur/prod", "pas du json")
pub(f"{prefix}/compteur/prod", "[1, 2]")
time.sleep(1.0)
r.check("charge non objet ignoree", scalar("SELECT state->>'power' FROM device WHERE friendly_name = 'compteur/prod'") == "1600")

# --- desactivation depuis la page : les messages suivants sont ignores ---------------------------------------------------------
click_in_row(name, ".crud-edit")
m.wait_for("document.querySelector('#crud-field-active') !== null", 5)
save_enabled()
m.js("document.querySelector('#crud-field-active').click()")
m.click("#crud-save")
r.check("source desactivee en base", wait_until(lambda: scalar(f"SELECT active::text FROM mqtt_source WHERE name = '{name}'") == "false"))
time.sleep(1.5)
pub(f"{prefix}/compteur/prod", {"power": 9})
pub(f"{prefix}/autre", {"power": 9})
time.sleep(1.5)
r.check("source inactive : messages ignores",
        scalar("SELECT state->>'power' FROM device WHERE friendly_name = 'compteur/prod'") == "1600"
        and scalar("SELECT count(*) FROM device WHERE friendly_name = 'autre'") == "0")

# --- suppression -----------------------------------------------------------------------------------------------------------------
click_in_row(name, ".crud-delete")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression de la source", wait_until(lambda: scalar(f"SELECT count(*) FROM mqtt_source WHERE name = '{name}'") == "0"))
r.check("l'appareil cree par la source reste", scalar("SELECT count(*) FROM device WHERE friendly_name = 'compteur/prod'") == "1")

r.finish()
