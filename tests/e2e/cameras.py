#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de l'administration des cameras (R6 lot 2) : ajout avec mot de passe (jamais reaffiche), path cree dans le MediaMTX de la
stack de dev, modification sans ressaisir le secret, synchronisation, page de consultation, suppression (path retire).

Usage : python3 tests/e2e/cameras.py url utilisateur_admin mot_de_passe
Variable : MEDIAMTX_API_URL (defaut http://127.0.0.1:19997, le MediaMTX de dev).
"""
import json
import os
import sys
import time
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: cameras.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
api = os.environ.get("MEDIAMTX_API_URL", "http://127.0.0.1:19997")
tag = f"E2E-{int(time.time())}"
secret = "secret-e2e-camera"

m = Marionette()
r = Report()


def mediamtx_paths():
    with urllib.request.urlopen(f"{api}/v3/config/paths/list?itemsPerPage=500", timeout=5) as resp:
        return {i["name"]: i for i in json.load(resp)["items"]}


def camera_paths():
    return {n: p for n, p in mediamtx_paths().items() if len(n) == 32 and all(c in "0123456789abcdef" for c in n)}


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def rows():
    return m.js("return [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent)") or []


def wait_rows(predicate_js, timeout=8):
    return m.wait_for(
        "(() => { const rows = [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent);"
        f" return {predicate_js}; }})()",
        timeout,
    )


def save_enabled():
    return m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)


def click_in_row(name, selector):
    return m.js(
        "const tr = [...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === "
        f"{name!r}); if (!tr) return false; tr.querySelector({selector!r}).click(); return true;"
    )


def open_form():
    for _ in range(40):  # until the interface is hydrated, the click opens nothing
        m.click("#crud-add")
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


before = set(camera_paths())

m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
m.goto(url + "admin/cameras")
r.check("page rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))

# Adding with a password -----------------------------------------------------------------------------------------
name = f"{tag}-Entree"
r.check("le formulaire s'ouvre", open_form())
save_enabled()
r.check("champ mot de passe masque", m.js("return document.querySelector('#crud-field-rtsp_pass').type") == "password")
fill("#crud-field-name", name)
fill("#crud-field-ip", "127.0.0.1")
fill("#crud-field-port", "1")
fill("#crud-field-path", "e2e")
fill("#crud-field-rtsp_user", "admin")
fill("#crud-field-rtsp_pass", secret)
m.click("#crud-save")
r.check("ajout : ligne dans le tableau", wait_rows(f"rows.includes({name!r})"), str(rows()))
created = set(camera_paths()) - before
r.check("un path de camera est cree dans MediaMTX", len(created) == 1, str(created))
path_name = next(iter(created), "")
conf = mediamtx_paths().get(path_name, {})
r.check("la source du path porte l'identifiant et le mot de passe",
        conf.get("source") == f"rtsp://admin:{secret}@127.0.0.1:1/e2e", "source differente")
r.check("le path est toujours ouvert (pas a la demande)", conf.get("sourceOnDemand") is False)

# The secret never comes back -------------------------------------------------------------------------------------
html = m.js("return document.documentElement.outerHTML")
r.check("le mot de passe n'apparait pas dans la page", secret not in html)
r.check("l'identifiant public s'affiche (32 hexadecimaux)", path_name in html)
click_in_row(name, ".crud-edit")
r.check("formulaire prerempli", m.wait_for(f"document.querySelector('#crud-field-name')?.value === {name!r}", 5))
r.check("le jeton du profil ONVIF est modifiable", m.js("return document.querySelector('#crud-field-main_token')?.type") == "text")
r.check("le mot de passe n'est pas prerempli", m.js("return document.querySelector('#crud-field-rtsp_pass').value") == "")

# Editing without re-entering the secret ---------------------------------------------------------------------------
save_enabled()
fill("#crud-field-fps", "15")
m.js("document.querySelector('#crud-field-rtsp_transport_tcp').click()")
m.click("#crud-save")
m.wait_for("document.querySelector('#crud-form') === null", 8)
conf = mediamtx_paths().get(path_name, {})
r.check("secret conserve apres modification", conf.get("source") == f"rtsp://admin:{secret}@127.0.0.1:1/e2e", str(conf.get("source", "")[:20]))
r.check("TCP force dans MediaMTX", conf.get("sourceProtocol") == "tcp", str(conf.get("sourceProtocol")))

# On-demand synchronisation: an unknown camera path disappears, the others stay ----------------------------------------
stray = "ab" * 16
req = urllib.request.Request(f"{api}/v3/config/paths/add/{stray}", data=json.dumps({"source": "rtsp://127.0.0.1:1/x"}).encode(),
                             headers={"Content-Type": "application/json"}, method="POST")
urllib.request.urlopen(req, timeout=5).read()
m.click("#cameras-sync")
r.check("bilan de synchronisation affiche", m.wait_for("document.querySelector('#cameras-sync-result') !== null", 8),
        m.text("#cameras-sync-result") or "")
r.check("le path inconnu est retire", stray not in mediamtx_paths())
r.check("le path de la camera reste", path_name in mediamtx_paths())
r.check("le path joker de la configuration reste", "~.*" in mediamtx_paths())

# Viewing by a logged-in account ----------------------------------------------------------------------------------
m.goto(url + "surveillance/")
r.check("la page de consultation liste la camera", m.wait_for(f"document.querySelector('[data-camera=\"{path_name}\"]') !== null", 10))
r.check("sans adresse IP ni chemin", "127.0.0.1" not in m.js("return document.querySelector('main').textContent"))

# Deletion ---------------------------------------------------------------------------------------------------------
m.goto(url + "admin/cameras")
m.wait_for("document.querySelector('#crud-add') !== null", 10)
m.wait_for(f"[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === {name!r})", 10)
for _ in range(40):  # until the interface is hydrated, the click opens nothing
    click_in_row(name, ".crud-delete")
    if m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 0.5):
        break
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression confirmee", wait_rows(f"!rows.includes({name!r})"), str(rows()))
r.check("le path est retire de MediaMTX", m.wait_for("true", 0.1) and path_name not in mediamtx_paths())

r.finish()
