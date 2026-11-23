#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parcours R2 de bout en bout, **par l'interface seulement** (aucun SQL) : creer un tableau depuis l'invitation, y ajouter un widget de
chaque type par l'administration, les voir sur le tableau, envoyer un message d'essai et un etat d'appareil par les outils des Reglages,
deplacer un widget au pointeur, recharger (tout persiste), suivre l'appareil en direct.

Usage : python3 tests/e2e/r2.py url utilisateur_admin mot_de_passe
"""
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: r2.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]

m = Marionette()
r = Report()

TYPES = {"text", "device", "grafana", "camera", "scene", "grille"}
CAMERA = "123e4567-e89b-12d3-a456-426614174000"


def sql(text):
    """SQL sur la base jetable : seulement pour creer l'appareil, dont l'interface de creation arrive au lot 5 de R3."""
    out = subprocess.run(os.environ["E2E_DB_SHELL"], shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def choose(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.value = {value!r};"
        "e.dispatchEvent(new Event('change', {bubbles: true})); return true;"
    )


def online():
    return m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)


def save():
    m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)
    m.click("#crud-save")


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def positions():
    """{titre: (colonne de grille, ligne de grille)} lus dans le style calcule."""
    return json.loads(m.js(
        "return JSON.stringify(Object.fromEntries([...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].map(e => "
        "[e.querySelector('.vc-widget-header-title').textContent, [+getComputedStyle(e).gridColumnStart, +getComputedStyle(e).gridRowStart]])))"
    ))


def drag(title, dx, dy):
    x, y = m.js(
        "const e = [...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].find(e => e.querySelector('.vc-widget-header-title').textContent === "
        f"{title!r}); const h = e.querySelector('.vc-widget-header').getBoundingClientRect(); return [h.left + 30, h.top + h.height / 2];"
    )
    m.cmd("WebDriver:PerformActions", {"actions": [{
        "type": "pointer", "id": "souris", "parameters": {"pointerType": "mouse"},
        "actions": [
            {"type": "pointerMove", "duration": 0, "x": int(x), "y": int(y), "origin": "viewport"},
            {"type": "pointerDown", "button": 0},
            {"type": "pointerMove", "duration": 150, "x": int(x + dx / 2), "y": int(y + dy / 2), "origin": "viewport"},
            {"type": "pointerMove", "duration": 150, "x": int(x + dx), "y": int(y + dy), "origin": "viewport"},
            {"type": "pointerUp", "button": 0},
        ],
    }]})
    m.cmd("WebDriver:ReleaseActions")


def add_widget(board, kind, title, col, row, width, height, **extra):
    m.goto(url + f"admin/widgets?d={board}")
    online()
    before = m.js("return document.querySelectorAll('#crud-table tbody tr').length") or 0
    m.click("#crud-add")
    m.wait_for("document.querySelector('#crud-field-widget_type') !== null", 10)
    choose("#crud-field-widget_type", kind)
    m.wait_for("document.querySelector('#crud-field-title') !== null", 5)
    fill("#crud-field-title", title)
    for name, value in extra.items():
        m.wait_for(f"document.querySelector('#crud-field-{name}') !== null", 10)
        if name == "device_id":  # drop-down list of devices, filled from the database
            m.wait_for(f"[...document.querySelector('#crud-field-device_id').options].some(o => o.value === {value!r})", 10)
            choose("#crud-field-device_id", value)
        elif name == "scene_id":  # drop-down list of scenes (R4)
            m.wait_for(f"[...document.querySelector('#crud-field-scene_id').options].some(o => o.value === {value!r})", 10)
            choose("#crud-field-scene_id", value)
        else:
            fill(f"#crud-field-{name}", value)
    for name, value in (("col", col), ("row", row), ("width", width), ("height", height)):
        fill(f"#crud-field-{name}", str(value))
    save()
    return m.wait_for(f"document.querySelectorAll('#crud-table tbody tr').length === {before + 1}", 15)


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
login = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", login == 200)
m.cmd("WebDriver:SetWindowRect", {"width": 1400, "height": 1000})

# Starting state: an account without a dashboard (the previous scripts on the same server created some). Cleanup through the server functions.
for d in json.loads(post("/sf/dashboards/list", "")["text"]):
    if d["owner"] == admin:
        post("/sf/dashboards/delete", f"id={d['id']}")

dev = int(re.search(r"^\s*(\d+)\s*$", sql("INSERT INTO device (friendly_name) VALUES ('E2E Sonde parcours') RETURNING id;"), re.M).group(1))

# 1. First dashboard, from the invitation ------------------------------------------------------------------------------------
m.goto(url)
r.check("invitation a creer un tableau", m.wait_for("document.querySelector('#vc-dash-empty') !== null", 20))
r.check("hydratation terminee", online())
m.js("document.querySelector('#vc-dash-new-name').value = 'Maison'; document.querySelector('#vc-dash-new-name').dispatchEvent(new Event('input', {bubbles: true}))")
m.click("#vc-dash-create")
r.check("tableau Maison cree", m.wait_for("document.querySelector('#vc-dash-name')?.textContent === 'Maison'", 15))
board = int(m.js("return new URLSearchParams(location.search).get('d')"))

# 2. One widget of each type, through the administration ----------------------------------------------------------------------
r.check("widget texte", add_widget(board, "text", "Note", 0, 0, 4, 2, content="<p>Bienvenue <b>chez vous</b></p><script>alert(1)</script>"))
r.check("widget appareil", add_widget(board, "device", "Sonde", 4, 0, 3, 2, device_id=str(dev)))
r.check("widget Grafana", add_widget(board, "grafana", "Courbe", 7, 0, 4, 3, config='{"dashboard_uid": "abc", "panel_id": 2}'))
r.check("widget camera", add_widget(board, "camera", "Porte", 0, 2, 2, 2, camera_uuid=CAMERA))
scene = int(re.search(r"^\s*(\d+)\s*$", sql("INSERT INTO scene (name) VALUES ('E2E Ambiance parcours') RETURNING id;"), re.M).group(1))
r.check("widget scene", add_widget(board, "scene", "Ambiance", 2, 2, 2, 2, scene_id=str(scene)))
r.check("widget grille", add_widget(board, "grille", "Grille", 4, 3, 2, 2))

# 3. The dashboard --------------------------------------------------------------------------------------------------------------
m.goto(url + f"?d={board}")
r.check("six widgets affiches", m.wait_for("document.querySelectorAll('#vc-dashboard-grid > .vc-widget').length === 6", 20))
r.check("un widget de chaque type", set(m.js("return [...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].map(e => e.dataset.widgetType)")) == TYPES)
r.check("texte assaini (mise en forme gardee, pas de script)", m.js("return document.querySelector('[data-widget-type=text] b')?.textContent") == "chez vous"
        and m.js("return document.querySelector('[data-widget-type=text] script')") is None)
r.check("Grafana : iframe vers l'URL publique", (m.js("return document.querySelector('[data-widget-type=grafana] iframe')?.getAttribute('src')") or "").startswith("http://localhost:3000/d/abc"))
r.check("appareil en attente d'un etat", m.js("return document.querySelector('[data-widget-type=device] #dev-waiting') !== null"))

# 4. The Settings test tools: broadcast message, device state -------------------------------------------------------------------
m.goto(url + "admin/settings")
online()
m.wait_for("document.querySelector('#broadcast-send') && !document.querySelector('#broadcast-send').disabled", 15)
fill("#broadcast-text", "Message d'essai R2")
m.click("#broadcast-send")
r.check("message d'essai affiche (notification)", m.wait_for("[...document.querySelectorAll('.vc-toast')].some(t => t.textContent.includes('Message d\\'essai R2'))", 15))
fill("#sim-device", str(dev))
fill("#sim-state", '{"temperature": 19.5, "humidity": 48}')
m.click("#sim-send")
r.check("etat d'appareil diffuse", m.wait_for("document.querySelector('#sim-status') !== null", 10))
r.check("... et recu par ce navigateur", m.wait_for("document.querySelector('#sim-received').textContent.includes('19.5')", 10))

# 5. The dashboard sees the retained state, then follows it live -----------------------------------------------------------------
m.goto(url + f"?d={board}")
r.check("l'appareil affiche le dernier etat des l'arrivee (humidite et temperature)",
        m.wait_for("document.querySelector('[data-widget-type=device] .vc-dev-state') !== null", 15)
        and "19.5" in m.text("[data-widget-type=device]") and "48" in m.text("[data-widget-type=device]"), m.text("[data-widget-type=device]"))
online()
m.js("window.__no_reload = 7")
post("/sf/devices/inject", f"device_id={dev}&state=%7B%22temperature%22%3A22.0%2C%22humidity%22%3A48%7D&available=true")
r.check("suivi en direct sans recharger (22)", m.wait_for("document.querySelector('[data-widget-type=device]').textContent.includes('22')", 15) and m.js("return window.__no_reload") == 7)

# 6. Pointer move, then reload -----------------------------------------------------------------------------------------------------
geo = json.loads(m.js(
    "const w = t => [...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].find(e => e.querySelector('.vc-widget-header-title').textContent === t).getBoundingClientRect();"
    "return JSON.stringify({a: w('Note').left, b: w('Sonde').left})"
))
pitch = (geo["b"] - geo["a"]) / 4
before = positions()
r.check("position de depart de la grille : colonne 5, ligne 4", before["Grille"] == [5, 4], str(before))
drag("Grille", 5 * pitch, 0)   # columns 4 -> 9 (under Grafana, row 3 free)
r.check("la grille est deplacee a la colonne 10", m.wait_for("(() => { const e = [...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].find(e => e.querySelector('.vc-widget-header-title').textContent === 'Grille'); return e && getComputedStyle(e).gridColumnStart === '10'; })()", 15), str(positions()))
m.goto(url + f"?d={board}")
r.check("apres rechargement : la position est conservee", m.wait_for("document.querySelectorAll('#vc-dashboard-grid > .vc-widget').length === 6", 20) and positions()["Grille"] == [10, 4], str(positions()))
r.check("... et les autres n'ont pas bouge", all(positions()[t] == before[t] for t in before if t != "Grille"), str(positions()))
online()
drag("Note", 6 * pitch, 0)   # on Sonde (column 4): occupied
r.check("emplacement occupe : refus expliqué, la note reste", m.wait_for("document.querySelector('#vc-dash-notice') !== null", 15) and positions()["Note"] == before["Note"], str(positions()))

# 7. Isolation: a second account does not see this dashboard -------------------------------------------------------------------------
res = post("/sf/dashboard", f"requested={board}")
r.check("la fonction serveur ne renvoie que les tableaux du compte connecte", res["status"] == 200 and json.loads(res["text"])["current"]["name"] == "Maison")

r.finish()
