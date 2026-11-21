#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de l'edition des tableaux dans Firefox : deplacement d'un widget au pointeur (persistant, refus d'un emplacement occupe),
administration des widgets (champs selon le type, chevauchement, suppression) et des tableaux.

Usage : python3 tests/e2e/dashboard_edit.py url utilisateur_admin mot_de_passe
Variable E2E_DB_SHELL (posee par tools/e2e.sh) : SQL sur l'entree standard, pour preparer les widgets de depart.
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: dashboard_edit.py url utilisateur_admin mot_de_passe")
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


def choose(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.value = {value!r};"
        "e.dispatchEvent(new Event('change', {bubbles: true})); return true;"
    )


def post(path, body, as_json=False):
    ctype = "application/json" if as_json else "application/x-www-form-urlencoded"
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':{ctype!r}}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def widgets(board):
    res = post("/sf/widgets/list", f"dashboard_id={board}")
    return {w["title"]: (w["col"], w["row"]) for w in json.loads(res["text"])}


def drag(widget_selector, dx, dy):
    """Glisse l'en-tete d'un widget de (dx, dy) pixels avec de vrais evenements de pointeur."""
    box = m.js(
        f"const h = document.querySelector({widget_selector!r} + ' .vc-widget-header').getBoundingClientRect();"
        "return [h.left + 30, h.top + h.height / 2];"
    )
    x, y = box
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


def save():
    """Clique sur Enregistrer une fois le bouton redevenu actif (pas d'envoi en cours)."""
    m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)
    m.click("#crud-save")


def online():
    return m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)


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

created = post("/sf/dashboard/create", "name=Edition")
board = json.loads(created["text"])["Saved"]["id"]
sql(f"""
INSERT INTO widget (dashboard_id, widget_type, title, grid_col, grid_row, width, height) VALUES
 ({board}, 'grille', 'A', 0, 0, 2, 2),
 ({board}, 'grille', 'B', 4, 0, 2, 2),
 ({board}, 'grille', 'C', 0, 3, 3, 2);
""")

# --- deplacement au pointeur --------------------------------------------------------------------------------------------------
m.goto(url + f"?d={board}")
r.check("trois widgets affiches", m.wait_for("document.querySelectorAll('#vc-dashboard-grid > .vc-widget').length === 3", 20))
r.check("hydratation terminee", online())
r.check("administrateur : les widgets sont deplacables", m.js("return document.querySelectorAll('.vc-widget--editable').length") == 3)
geo = json.loads(m.js(
    "const w = [...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')];"
    "const by = n => w.find(e => e.querySelector('.vc-widget-header-title').textContent === n).getBoundingClientRect();"
    "return JSON.stringify({a: by('A').left, b: by('B').left, aTop: by('A').top, c: by('C').top})"
))
pitch = (geo["b"] - geo["a"]) / 4   # largeur d'une colonne + intervalle
row_pitch = (geo["c"] - geo["aTop"]) / 3
m.js("[...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].forEach(e => { e.dataset.t = e.querySelector('.vc-widget-header-title').textContent; })")
A = "[data-t=A]"
drag(A, 6 * pitch, 0)
r.check("A deplace de 6 colonnes (colonne 7 de la grille) apres relachement",
        m.wait_for("[...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].some(e => e.querySelector('.vc-widget-header-title').textContent === 'A' && getComputedStyle(e).gridColumnStart === '7')", 15),
        str(widgets(board)))
r.check("deplacement enregistre en base", widgets(board)["A"] == (6, 0), str(widgets(board)))
r.check("le cadre en pointilles est masque", m.js("return getComputedStyle(document.querySelector('#vc-drop-ghost')).display") == "none")
r.check("plus de transformation restante", m.js("return [...document.querySelectorAll('.vc-widget')].every(e => !e.style.transform)"))
m.goto(url + f"?d={board}")
r.check("position conservee apres rechargement", m.wait_for("[...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].some(e => e.querySelector('.vc-widget-header-title')?.textContent === 'A' && getComputedStyle(e).gridColumnStart === '7')", 15))

# --- emplacement occupe : refuse, le widget revient --------------------------------------------------------------------------
online()
m.js("[...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].forEach(e => { e.dataset.t = e.querySelector('.vc-widget-header-title').textContent; })")
drag("[data-t=A]", -2 * pitch, 0)   # de la colonne 6 a la colonne 4 : sur B
r.check("emplacement occupe : message d'erreur", m.wait_for("document.querySelector('#vc-dash-notice') !== null", 15), m.text("#vc-dash-notice"))
r.check("... et le widget n'a pas bouge en base", widgets(board)["A"] == (6, 0), str(widgets(board)))
r.check("... ni a l'ecran", m.js("return getComputedStyle([...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].find(e => e.querySelector('.vc-widget-header-title').textContent === 'A')).gridColumnStart") == "7")
drag("[data-t=C]", 0, row_pitch)
r.check("deplacement vertical (ligne 3 -> 4)", m.wait_for("[...document.querySelectorAll('#vc-dashboard-grid > .vc-widget')].some(e => e.querySelector('.vc-widget-header-title')?.textContent === 'C' && getComputedStyle(e).gridRowStart === '5')", 15), str(widgets(board)))
r.check("... et le message d'erreur precedent a disparu", m.js("return document.querySelector('#vc-dash-notice')") is None)

# --- administration des widgets ------------------------------------------------------------------------------------------------
m.goto(url + f"admin/widgets?d={board}")
r.check("page des widgets : le tableau est choisi", m.wait_for(f"document.querySelector('#widgets-dashboard')?.value === '{board}'", 20))
r.check("trois lignes", m.wait_for("document.querySelectorAll('#crud-table tbody tr').length === 3", 15))
online()  # hydratation terminee : sinon le clic ne declenche rien
m.click("#crud-add")
r.check("formulaire : type 'texte' par defaut -> champ contenu, pas de configuration", m.wait_for("document.querySelector('#crud-field-content') !== null", 10)
        and m.js("return document.querySelector('#crud-field-config')") is None and m.js("return document.querySelector('#crud-field-device_id')") is None)
choose("#crud-field-widget_type", "grafana")
r.check("type grafana -> configuration et appareil", m.wait_for("document.querySelector('#crud-field-config') !== null && document.querySelector('#crud-field-device_id') !== null && document.querySelector('#crud-field-content') === null", 10))
choose("#crud-field-widget_type", "scene")
r.check("type scene -> liste des scenes seulement", m.wait_for("document.querySelector('#crud-field-scene_id') !== null && document.querySelector('#crud-field-config') === null", 10))
choose("#crud-field-widget_type", "text")
m.wait_for("document.querySelector('#crud-field-content') !== null", 10)
fill("#crud-field-title", "Nouveau")
fill("#crud-field-content", "<p>Salut <b>toi</b></p><script>alert(1)</script>")
fill("#crud-field-col", "4")
fill("#crud-field-row", "0")
save()
r.check("chevauchement : erreur sur la colonne", m.wait_for("document.querySelector('#crud-error-col') !== null", 10), m.text("#crud-error-col"))
fill("#crud-field-col", "0")
fill("#crud-field-row", "8")
save()
r.check("widget cree", m.wait_for("document.querySelectorAll('#crud-table tbody tr').length === 4", 10),
        m.js("return JSON.stringify({loadErr: !!document.querySelector('#crud-load-error'), alerts: [...document.querySelectorAll('.vc-alert')].map(a => a.textContent), tables: document.querySelectorAll('#crud-table').length, rowTitles: [...document.querySelectorAll('#crud-table tbody tr')].map(t => t.querySelector('td').textContent), rows: document.querySelectorAll('#crud-table tbody tr').length, form: !!document.querySelector('#crud-form'),"
             " errCol: document.querySelector('#crud-error-col')?.textContent, errAny: [...document.querySelectorAll('.vc-field-error')].map(e => e.id + ':' + e.textContent),"
             " banner: document.querySelector('#crud-banner')?.textContent, col: document.querySelector('#crud-field-col')?.value, row: document.querySelector('#crud-field-row')?.value})")
        + " db=" + str(sorted(widgets(board))))
saved = json.loads(post("/sf/widgets/list", f"dashboard_id={board}")["text"])
new = next(w for w in saved if w["title"] == "Nouveau")
r.check("HTML assaini a l'enregistrement (pas de script)", "<script" not in new["content"] and "<b>toi</b>" in new["content"], new["content"])
# Suppression avec confirmation.
m.js("[...document.querySelectorAll('#crud-table tbody tr')].find(t => t.textContent.includes('Nouveau')).querySelector('.crud-delete').click()")
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("widget deleted", m.wait_for("document.querySelectorAll('#crud-table tbody tr').length === 3", 10))

# --- administration des tableaux ---------------------------------------------------------------------------------------------------
m.goto(url + "admin/dashboards")
r.check("page des tableaux", m.wait_for("document.querySelector('#crud-table') !== null", 20))
online()  # hydratation terminee : sinon le clic ne declenche rien
m.click("#crud-add")
m.wait_for("document.querySelector('#crud-field-owner') !== null", 10)
fill("#crud-field-name", "Garage")
fill("#crud-field-owner", "personne-ici")
save()
r.check("proprietaire inconnu refuse", m.wait_for("document.querySelector('#crud-error-owner') !== null", 10))
fill("#crud-field-owner", admin)
save()
r.check("tableau cree pour l'administrateur", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr')].some(t => t.textContent.includes('Garage'))", 10))
m.goto(url)
r.check("le nouveau tableau apparait dans le selecteur de l'accueil", m.wait_for("[...document.querySelectorAll('#vc-dash-picker .vc-dash-picker-item')].some(a => a.textContent.includes('Garage'))", 15))

r.finish()
