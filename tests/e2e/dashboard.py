#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E du tableau de bord (page d'accueil) dans Firefox : invitation a creer un tableau, creation, placement des widgets par
classes (CSP sans style en ligne), texte assaini, appareil suivi en direct, Grafana, selecteur, petit ecran.

Usage : python3 tests/e2e/dashboard.py url utilisateur_admin mot_de_passe
Variable d'environnement E2E_DB_SHELL (posee par tools/e2e.sh) : commande qui lit du SQL sur son entree standard ; elle prepare
les widgets, que l'interface ne sait pas encore creer (edition au lot 5).
"""
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: dashboard.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
login = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", login == 200)

# --- compte sans tableau : invitation, puis creation par l'interface -------------------------------------------------
m.goto(url)
r.check("invitation a creer un premier tableau", m.wait_for("document.querySelector('#vc-dash-empty') !== null", 20))
r.check("aucune grille tant qu'il n'y a pas de tableau", m.js("return document.querySelector('#vc-dashboard-grid')") is None)
r.check("hydratation terminee (temps reel connecte)", m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20))
fill("#vc-dash-new-name", "x" * 65)
m.js("document.querySelector('#vc-dash-new-name').removeAttribute('maxlength')")
fill("#vc-dash-new-name", "x" * 65)
m.click("#vc-dash-create")
r.check("nom trop long : message d'erreur", m.wait_for("document.querySelector('#vc-dash-error') !== null", 10), m.text("#vc-dash-error"))
fill("#vc-dash-new-name", "Salon")
m.click("#vc-dash-create")
r.check("tableau cree : on ouvre ?d=<id>", m.wait_for("/^\\?d=\\d+$/.test(location.search)", 10), m.js("return location.href"))
r.check("titre du tableau", m.wait_for("document.querySelector('#vc-dash-name')?.textContent === 'Salon'", 10), m.text("#vc-dash-name"))
r.check("tableau vide : message", m.js("return document.querySelector('#vc-dash-no-widgets') !== null"))
board = int(m.js("return new URLSearchParams(location.search).get('d')"))

# --- widgets (prepares en SQL), puis rendu serveur ----------------------------------------------------------------------
# Depuis R3 un widget vise un appareil qui existe (cle etrangere) ; l'interface de creation des appareils arrive au lot 5.
dev = int(re.search(r"^\s*(\d+)\s*$", sql("INSERT INTO device (friendly_name) VALUES ('E2E Sonde tableau') RETURNING id;"), re.M).group(1))
# Idem pour la scene (cle etrangere depuis R4) : creee avant le widget.
scene = int(re.search(r"^\s*(\d+)\s*$", sql("INSERT INTO scene (name) VALUES ('E2E Ambiance tableau') RETURNING id;"), re.M).group(1))
sql(f"""
INSERT INTO widget (dashboard_id, widget_type, title, device_id, grid_col, grid_row, width, height, config, content) VALUES
 ({board}, 'text',    'Note',     NULL, 0, 0, 4, 2, '{{}}',
  '<p id="note-p">Bonjour <b>vous</b></p><script>window.__pwned = 1</script><img src="x" onerror="window.__pwned = 2"><a href="javascript:window.__pwned = 3">lien</a>'),
 ({board}, 'device',  'Sonde',    {dev},   6, 0, 3, 2, '{{}}', ''),
 ({board}, 'grille',  '',         NULL, 0, 2, 2, 2, '{{}}', ''),
 ({board}, 'grafana', 'Courbe',   NULL, 4, 2, 4, 3, '{{"dashboard_uid": "abc", "panel_id": 2}}', ''),
 ({board}, 'grafana', 'Ailleurs', NULL, 8, 2, 4, 3, '{{"panel_url": "http://autre.invalid/d-solo/x"}}', ''),
 ({board}, 'camera',  'Porte',    NULL, 0, 5, 2, 2, '{{}}', ''),
 ({board}, 'scene',   '',         NULL, 2, 5, 2, 2, '{{}}', '');
UPDATE widget SET camera_uuid = '123e4567-e89b-12d3-a456-426614174000' WHERE widget_type = 'camera' AND dashboard_id = {board};
UPDATE widget SET scene_id = {scene} WHERE widget_type = 'scene' AND dashboard_id = {board};
""")
m.goto(url + f"?d={board}")
r.check("la grille est rendue", m.wait_for("document.querySelector('#vc-dashboard-grid') !== null", 20))
r.check("7 widgets", m.js("return document.querySelectorAll('#vc-dashboard-grid > .vc-widget').length") == 7)
info = json.loads(m.js(
    "const q = t => document.querySelector(`[data-widget-type=${t}]`);"
    "const note = q('text'), dev = q('device');"
    "const cs = e => getComputedStyle(e);"
    "return JSON.stringify({noteStart: cs(note).gridColumnStart, devStart: cs(dev).gridColumnStart, devRowStart: cs(dev).gridRowStart,"
    " noteW: note.getBoundingClientRect().width, devW: dev.getBoundingClientRect().width, noteLeft: note.getBoundingClientRect().left,"
    " devLeft: dev.getBoundingClientRect().left, noteH: note.getBoundingClientRect().height})"
))
r.check("colonne de depart issue de la classe (col 0 -> 1, col 6 -> 7)", info["noteStart"] == "1" and info["devStart"] == "7", str(info))
r.check("largeurs proportionnelles (4 colonnes contre 3)", 1.1 < info["noteW"] / info["devW"] < 1.5, str(info))
r.check("le widget de droite est a droite de la note", info["devLeft"] > info["noteLeft"] + info["noteW"] * 0.9, str(info))
r.check("aucun attribut style rendu par le serveur", m.js("return document.querySelectorAll('#vc-dashboard-grid [style]').length") == 0)

# --- texte assaini --------------------------------------------------------------------------------------------------------
r.check("mise en forme conservee", m.js("return document.querySelector('[data-widget-type=text] .vc-widget-text p b')?.textContent") == "vous")
r.check("script, gestionnaire et lien javascript: neutralises", m.js("return window.__pwned === undefined"), str(m.js("return window.__pwned")))
r.check("aucun lien javascript: dans la page", m.js("return [...document.querySelectorAll('a')].every(a => !a.href.startsWith('javascript:'))"))

# --- Grafana ------------------------------------------------------------------------------------------------------------
frame = m.js("return document.querySelector('[data-widget-type=grafana] iframe')?.getAttribute('src')")
r.check("Grafana : iframe vers l'URL publique par defaut", (frame or "").startswith("http://localhost:3000/d/abc?viewPanel=2"), str(frame))
r.check("Grafana : sandbox sur l'iframe", (m.js("return document.querySelector('[data-widget-type=grafana] iframe')?.getAttribute('sandbox')") or "") != "")
r.check("Grafana : adresse d'une autre origine refusee (pas d'iframe)", m.js("return document.querySelectorAll('[data-widget-type=grafana] iframe').length") == 1
        and "autre.invalid" not in m.js("return document.documentElement.outerHTML"))
csp = m.js_async("const cb = arguments[arguments.length-1]; fetch(location.href).then(x => cb(x.headers.get('content-security-policy')));")
r.check("CSP : frame-src limite a l'origine de Grafana", "frame-src http://localhost:3000" in (csp or ""), str(csp)[:200])

# --- appareil (propre a ce script : le serveur retient le dernier etat de chaque appareil) : attente, puis direct --------------------------------------------------------------------------
r.check("appareil sans etat : attente", m.wait_for("document.querySelector('[data-widget-type=device] #dev-waiting') !== null", 10))
m.js("window.__no_reload = 42")
res = post("/sf/devices/inject", f"device_id={dev}&state=" + '{"temperature":21.5}'.replace('"', "%22").replace("{", "%7B").replace("}", "%7D").replace(":", "%3A").replace(",", "%2C") + "&available=true")
r.check("etat injecte (administrateur)", res["status"] == 200, str(res))
r.check("l'appareil affiche la valeur sans recharger (21.5 °C)",
        m.wait_for("document.querySelector('[data-widget-type=device] .vc-dev-primary-val')?.textContent === '21.5'", 15),
        m.text("[data-widget-type=device]"))
res = post("/sf/devices/inject", f"device_id={dev}&state=%7B%22temperature%22%3A23.1%7D&available=true")
r.check("nouvelle valeur suivie en direct (23.1)", m.wait_for("document.querySelector('[data-widget-type=device] .vc-dev-primary-val')?.textContent === '23.1'", 15))
r.check("sans rechargement de la page", m.js("return window.__no_reload") == 42)
post("/sf/devices/inject", f"device_id={dev}&state=%7B%22state%22%3A%22ON%22%2C%22temperature%22%3A23.1%7D&available=true")
r.check("appareil allume : cadre vert", m.wait_for("document.querySelector('[data-widget-type=device]').classList.contains('vc-widget--on')", 10))
post("/sf/devices/inject", f"device_id={dev}&state=%7B%22state%22%3A%22ON%22%7D&available=false")
r.check("appareil indisponible : cadre attenue", m.wait_for("document.querySelector('[data-widget-type=device]').classList.contains('vc-widget--offline')", 10))
post("/sf/devices/inject", f"device_id={dev}&state=%7B%22state%22%3A%22OFF%22%2C%22power%22%3A12%7D&available=true")
r.check("commutateur : commandes actives (le serveur a un client MQTT et l'appareil est disponible)",
        m.wait_for("document.querySelectorAll('[data-widget-type=device] .vc-dev-ctrl').length === 2 && document.querySelectorAll('[data-widget-type=device] .vc-dev-ctrl[disabled]').length === 0", 10))
m.goto(url + f"?d={board}")
r.check("rendu serveur : dernier etat connu des l'arrivee", m.wait_for("document.querySelector('[data-widget-type=device] .vc-dev-state') !== null", 10) and "power" in m.text("[data-widget-type=device]").lower(), m.text("[data-widget-type=device]"))

# --- camera : un widget sur une camera inconnue le dit (R6) ; scene : nom et bouton d'activation (R4) ---------------------------------------------
r.check("camera inconnue : le widget le dit", m.js("return document.querySelectorAll('[id^=widget-camera-missing-]').length") == 1)
r.check("scene : nom et bouton d'activation", "E2E Ambiance tableau" in (m.text("[data-widget-type=scene]") or "")
        and m.js("return document.querySelector('[data-widget-type=scene] .scene-activate') !== null"), m.text("[data-widget-type=scene]"))

# --- selecteur de tableaux ------------------------------------------------------------------------------------------------
r.check("un seul tableau : pas de selecteur", m.js("return document.querySelector('#vc-dash-picker')") is None)
res = post("/sf/dashboard/create", "name=Cave")
r.check("second tableau cree", res["status"] == 200 and "Saved" in res["text"], str(res))
m.goto(url + f"?d={board}")
r.check("deux tableaux : selecteur affiche", m.wait_for("document.querySelector('#vc-dash-picker') !== null", 10))
r.check("temps reel connecte (hydratation terminee)", m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20))
m.js("window.__no_reload = 43")
m.js("document.querySelector('#vc-dash-picker summary').click()")
r.check("le menu liste les deux tableaux", m.js("return document.querySelectorAll('#vc-dash-picker .vc-dash-picker-item').length") == 2)
m.js("[...document.querySelectorAll('#vc-dash-picker .vc-dash-picker-item')].find(a => a.textContent.includes('Cave')).click()")
r.check("navigation vers Cave sans rechargement", m.wait_for("document.querySelector('#vc-dash-name')?.textContent === 'Cave'", 10) and m.js("return window.__no_reload") == 43,
        m.text("#vc-dash-name"))
r.check("l'autre tableau n'a pas de widget", m.js("return document.querySelector('#vc-dash-no-widgets') !== null"))
m.goto(url + "?d=999999")
r.check("tableau inexistant : retour au tableau par defaut", m.wait_for("document.querySelector('#vc-dash-name')?.textContent === 'Salon'", 10), m.text("#vc-dash-name"))

# --- petit ecran ----------------------------------------------------------------------------------------------------------
m.cmd("WebDriver:SetWindowRect", {"width": 480, "height": 900})
m.goto(url + f"?d={board}")
m.wait_for("document.querySelector('#vc-dashboard-grid') !== null", 15)
over = m.js("return document.documentElement.scrollWidth - window.innerWidth")
r.check("petit ecran : pas de defilement horizontal", over <= 1, str(over))
r.check("petit ecran : les widgets passent sur toute la largeur", m.js(
    "const g = document.querySelector('#vc-dashboard-grid').getBoundingClientRect(); const w = document.querySelector('[data-widget-type=grille]').getBoundingClientRect();"
    "return w.width > g.width * 0.4") is True)
m.cmd("WebDriver:SetWindowRect", {"width": 1280, "height": 900})

r.finish()
