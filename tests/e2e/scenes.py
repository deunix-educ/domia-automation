#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des scenes dans Firefox (R4 lot 2) : administration (scene + commandes ordonnees, appareil en liste, JSON valide), page
/automation/scenes, activation qui publie les commandes DANS L'ORDRE sur le broker de dev, retour ✓ / ✗, widget scene du tableau de
bord (liste deroulante dans l'administration des widgets, bouton sur l'accueil).

Usage : python3 tests/e2e/scenes.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable). Le prefixe Zigbee2MQTT est remplace par un prefixe propre au script.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: scenes.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
base = f"e2escn{int(time.time())}"
dev_a, dev_b = "E2E Scene A", "E2E Scene B"

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def listen(topic, count):
    """Ecoute `count` messages (topic + charge) sur le broker, en tache de fond."""
    return subprocess.Popen(["docker", "exec", broker, "mosquitto_sub", "-h", "localhost", "-q", "1", "-v", "-C", str(count), "-W", "20", "-t", topic],
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, text=True)


def heard(proc):
    try:
        out, _ = proc.communicate(timeout=25)
    except subprocess.TimeoutExpired:
        proc.kill()
        return []
    return [line for line in out.strip().splitlines() if line]


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def row_fill(i, value):
    return m.js(
        f"const e = document.querySelectorAll('.vc-pairs-row')[{i}]?.querySelector('.vc-pairs-text'); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def row_choose(i, label):
    return m.js(
        f"const s = document.querySelectorAll('.vc-pairs-row')[{i}]?.querySelector('.vc-pairs-choice'); if (!s) return false;"
        f"const o = [...s.options].find(o => o.textContent === {label!r}); if (!o) return false;"
        "s.value = o.value; s.dispatchEvent(new Event('change', {bubbles: true})); return true;"
    )


def row_click(i, cls):
    return m.js(f"const b = document.querySelectorAll('.vc-pairs-row')[{i}]?.querySelector('.{cls}'); if (!b) return false; b.click(); return true;")


def open_form():
    for _ in range(40):  # tant que l'interface n'est pas hydratee, le clic n'ouvre rien
        m.click("#crud-add")
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


def save_enabled():
    return m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def commands(scene):
    return scalar(f"SELECT string_agg(d.friendly_name || '=' || (sa.payload->>'state'), ',' ORDER BY sa.position) FROM scene_action sa JOIN device d ON d.id = sa.device_id WHERE sa.scene_id = {scene}")


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
sql(f"INSERT INTO device (friendly_name, base_topic) VALUES ('{dev_a}', '{base}'), ('{dev_b}', '{base}');")

# --- administration : scene avec deux commandes ----------------------------------------------------------------------------
m.goto(url + "admin/scenes")
r.check("page des scenes (administration)", m.wait_for("document.querySelector('#crud-add') !== null", 10))
open_form()
save_enabled()
r.check("editeur de commandes affiche", m.wait_for("document.querySelector('.vc-pairs .pairs-add') !== null", 10))
r.check("liste d'emplacements remplie ou vide sans erreur", m.js("return document.querySelector('#crud-field-location_id') !== null"))
fill("#crud-field-name", "E2E Soiree")
fill("#crud-field-icon", "🌙")
m.click(".pairs-add")
m.click(".pairs-add")
r.check("deux lignes de commande", m.wait_for("document.querySelectorAll('.vc-pairs-row').length === 2", 10))
r.check("liste d'appareils remplie depuis la base",
        m.wait_for(f"[...(document.querySelector('.vc-pairs-choice')?.options || [])].some(o => o.textContent === {dev_a!r})", 10))
row_choose(0, dev_a)
row_fill(0, '{"state":"ON"}')
row_choose(1, dev_b)
row_fill(1, "pas du json")
m.click("#crud-save")
r.check("commande non JSON : erreur sur le champ", m.wait_for("document.querySelector('#crud-error-actions') !== null", 10), m.text("#crud-error-actions"))
r.check("... et rien n'est enregistre", scalar("SELECT count(*) FROM scene WHERE name = 'E2E Soiree'") == "0")
row_fill(1, '{"state":"OFF"}')
m.click("#crud-save")
r.check("scene creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Soiree')", 10))
scene = int(scalar("SELECT id FROM scene WHERE name = 'E2E Soiree'"))
r.check("commandes en base, dans l'ordre saisi", commands(scene) == f"{dev_a}=ON,{dev_b}=OFF", commands(scene))
r.check("colonne du nombre de commandes", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr')].some(t => t.textContent.includes('E2E Soiree') && t.textContent.includes('2'))", 5))

# --- modification : le formulaire est prerempli, on inverse l'ordre ------------------------------------------------------
m.js("[...document.querySelectorAll('#crud-table tbody tr')].find(t => t.textContent.includes('E2E Soiree')).querySelector('.crud-edit').click()")
r.check("formulaire prerempli : deux commandes", m.wait_for("document.querySelectorAll('.vc-pairs-row').length === 2", 10))
r.check("... avec leurs appareils et leurs textes",
        m.js("return [...document.querySelectorAll('.vc-pairs-row')].map(x => x.querySelector('.vc-pairs-choice').selectedOptions[0].textContent + '|' + x.querySelector('.vc-pairs-text').value).join(';')")
        == f'{dev_a}|{{"state":"ON"}};{dev_b}|{{"state":"OFF"}}')
row_click(0, "pairs-down")
r.check("ordre inverse a l'ecran", m.wait_for(f"document.querySelectorAll('.vc-pairs-row')[0]?.querySelector('.vc-pairs-choice').selectedOptions[0].textContent === {dev_b!r}", 5))
save_enabled()
m.click("#crud-save")
r.check("ordre inverse en base", m.wait_for("document.querySelector('#crud-form') === null", 10) and commands(scene) == f"{dev_b}=OFF,{dev_a}=ON", commands(scene))

# --- page des scenes et activation -------------------------------------------------------------------------------------------
m.goto(url + "automation/scenes")
r.check("carte de la scene", m.wait_for(f"document.querySelector('#scene-card-{scene}') !== null", 15))
r.check("... avec ses commandes dans l'ordre", m.js(f"return [...document.querySelectorAll('#scene-card-{scene} .vc-scene-device')].map(e => e.textContent).join(',')") == f"{dev_b},{dev_a}")
m.wait_for(f"document.querySelector('#scene-activate-{scene}') && !document.querySelector('#scene-activate-{scene}').disabled", 15)
sub = listen(f"{base}/#", 2)
time.sleep(1.0)
m.click(f"#scene-activate-{scene}")
got = heard(sub)
r.check("deux commandes publiees dans l'ordre de la scene",
        got == [f'{base}/{dev_b}/set {{"state":"OFF"}}', f'{base}/{dev_a}/set {{"state":"ON"}}'], str(got))
r.check("retour ✓", m.wait_for(f"(document.querySelector('#scene-result-{scene}')?.textContent || '').startsWith('✓')", 10), m.text(f"#scene-result-{scene}"))

# --- une commande en echec n'arrete pas la suivante -----------------------------------------------------------------------------
sql(f"UPDATE device SET active = FALSE WHERE friendly_name = '{dev_b}';")
sub = listen(f"{base}/#", 1)
time.sleep(1.0)
m.click(f"#scene-activate-{scene}")
got = heard(sub)
r.check("appareil inactif ignore, la commande suivante part", got == [f'{base}/{dev_a}/set {{"state":"ON"}}'], str(got))
r.check("retour ✗ avec le nom de l'appareil", m.wait_for(f"(document.querySelector('#scene-result-{scene}')?.textContent || '').startsWith('✗') && document.querySelector('#scene-result-{scene}').textContent.includes({dev_b!r})", 10),
        m.text(f"#scene-result-{scene}"))
sql(f"UPDATE device SET active = TRUE WHERE friendly_name = '{dev_b}';")

# --- widget scene : liste deroulante dans l'administration, bouton sur l'accueil ---------------------------------------------------
admin_id = scalar(f"SELECT id FROM users WHERE username = '{admin}'")
board = int(re.search(r"^\s*(\d+)\s*$", sql(f"INSERT INTO dashboard (name, owner_id) VALUES ('E2E Scenes', {admin_id}) RETURNING id;"), re.M).group(1))
m.goto(url + f"admin/widgets?d={board}")
r.check("page des widgets", m.wait_for("document.querySelector('#crud-add') !== null", 10))
open_form()
save_enabled()
m.js("const s = document.querySelector('#crud-field-widget_type'); s.value = 'scene'; s.dispatchEvent(new Event('change', {bubbles: true}));")
r.check("widget scene : liste deroulante des scenes",
        m.wait_for("[...(document.querySelector('#crud-field-scene_id')?.options || [])].some(o => o.textContent === 'E2E Soiree')", 10))
m.js("const s = document.querySelector('#crud-field-scene_id'); const o = [...s.options].find(o => o.textContent === 'E2E Soiree'); s.value = o.value; s.dispatchEvent(new Event('change', {bubbles: true}));")
fill("#crud-field-title", "Soiree")
m.click("#crud-save")
r.check("widget scene cree", m.wait_for("document.querySelector('#crud-form') === null", 10) and scalar(f"SELECT scene_id FROM widget WHERE dashboard_id = {board}") == str(scene))
m.goto(url + f"?d={board}")
r.check("widget scene affiche", m.wait_for("document.querySelector('[data-widget-type=scene] .scene-activate') !== null", 15))
m.wait_for("!document.querySelector('[data-widget-type=scene] .scene-activate').disabled", 15)
sub = listen(f"{base}/#", 2)
time.sleep(1.0)
m.click("[data-widget-type=scene] .scene-activate")
got = heard(sub)
r.check("le bouton du widget publie les deux commandes dans l'ordre",
        got == [f'{base}/{dev_b}/set {{"state":"OFF"}}', f'{base}/{dev_a}/set {{"state":"ON"}}'], str(got))
r.check("retour ✓ sur le widget", m.wait_for("(document.querySelector('[data-widget-type=scene] .vc-scene-result')?.textContent || '').startsWith('✓')", 10))

# --- suppression : le widget le dit, la page aussi --------------------------------------------------------------------------------
m.goto(url + "admin/scenes")
m.wait_for("document.querySelector('#crud-add') !== null", 10)
m.wait_for("document.querySelectorAll('#crud-table tbody tr').length > 0", 10)
for _ in range(40):
    m.js("[...document.querySelectorAll('#crud-table tbody tr')].find(t => t.textContent.includes('E2E Soiree'))?.querySelector('.crud-delete')?.click()")
    if m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 0.5):
        break
m.click(".crud-confirm-yes")
r.check("scene deleted", m.wait_for("![...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Soiree')", 10)
        and scalar("SELECT count(*) FROM scene WHERE name = 'E2E Soiree'") == "0")
r.check("... et ses commandes aussi", scalar(f"SELECT count(*) FROM scene_action WHERE scene_id = {scene}") == "0")
r.check("... le widget perd sa scene", scalar(f"SELECT coalesce(scene_id::text, 'null') FROM widget WHERE dashboard_id = {board}") == "null")
m.goto(url + f"?d={board}")
r.check("le widget annonce une scene introuvable", m.wait_for("document.querySelector('[id^=scene-problem-]') !== null", 15))

# Nettoyage : la base jetable est partagee avec les scripts suivants (le tableau de l'administrateur, notamment).
sql(f"DELETE FROM dashboard WHERE id = {board}; DELETE FROM device WHERE friendly_name IN ('{dev_a}', '{dev_b}');")
sys.exit(r.finish())
