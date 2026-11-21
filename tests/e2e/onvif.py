#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page /surveillance/discovery (R6 lot 3) : message clair quand aucun role `discovery` ne repond, ajout d'une camera par
adresse IP (camera ONVIF simulee par tools/sim-onvif.py), synchronisation ONVIF, mot de passe jamais affiche, adresse invalide.

Usage : python3 tests/e2e/onvif.py url utilisateur_admin mot_de_passe
Variables : E2E_ONVIF_PORT, E2E_ONVIF_PASSWORD (poses par tools/e2e.sh).
"""
import os
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: onvif.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
port, secret = os.environ.get("E2E_ONVIF_PORT"), os.environ.get("E2E_ONVIF_PASSWORD")
if not port or not secret:
    sys.exit("E2E_ONVIF_PORT / E2E_ONVIF_PASSWORD absents : lancer par tools/e2e.sh")

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
try:
    m.goto(url + "surveillance/discovery")
    r.check("page rendue", m.wait_for("document.querySelector('#discovery-start') !== null", 10))

    # Aucun role `discovery` dans l'E2E : un message le dit au lieu d'attendre indefiniment.
    silent = False
    for _ in range(3):  # un clic avant l'hydratation ne fait rien : on recommence
        m.click("#discovery-start")
        if m.wait_for("document.querySelector('#discovery-silent') !== null", 9):
            silent = True
            break
    r.check("message clair sans role discovery", silent, m.text("#discovery-silent") or "")
    r.check("le message nomme la commande", "--only discovery" in (m.text("#discovery-silent") or ""))

    # Adresse invalide ---------------------------------------------------------------------------------------------
    fill("#manual-ip", "cam; rm -rf")
    m.click("#manual-submit")
    r.check("adresse invalide refusee", m.wait_for("(document.querySelector('#manual-result')?.textContent || '').includes('invalide')", 8),
            m.text("#manual-result") or "")

    # Ajout par adresse, avec identifiants : camera creee et synchronisee ------------------------------------------------
    fill("#manual-ip", "127.0.0.1")
    fill("#manual-port", port)
    fill("#manual-user", "admin")
    fill("#manual-pass", secret)
    m.click("#manual-submit")
    r.check("camera ajoutee et synchronisee",
            m.wait_for("(document.querySelector('#manual-result')?.textContent || '').includes('IPC_NT98566')", 10),
            m.text("#manual-result") or "")
    r.check("le mot de passe saisi est efface apres l'ajout", m.js("return document.querySelector('#manual-pass').value") == "")
    r.check("la camera apparait dans la synchronisation ONVIF",
            m.wait_for("document.querySelector('#onvif-cameras li') !== null", 8),
        (m.js("return document.querySelector('#onvif-cameras')?.innerHTML") or "")[:300])
    r.check("fabricant et modele affiches", "IPC_NT98566" in (m.text("#onvif-cameras .camera-info") or ""), m.text("#onvif-cameras .camera-info") or "")

    # Deuxieme ajout de la meme adresse --------------------------------------------------------------------------------
    m.click("#manual-submit")
    r.check("meme adresse : camera existante", m.wait_for("(document.querySelector('#manual-result')?.textContent || '').includes('déjà')", 8),
            m.text("#manual-result") or "")

    # Synchronisation ONVIF a la demande ------------------------------------------------------------------------------
    m.click(".camera-onvif-sync")
    r.check("synchronisation ONVIF : profil et PTZ",
            m.wait_for("(document.querySelector('.onvif-result')?.textContent || '').includes('Profile_1')", 10),
            m.text(".onvif-result") or "")
    shown = m.text(".onvif-result") or ""
    r.check("URL de flux masquee", "password=***" in shown and secret not in shown, shown)
    html = m.js("return document.documentElement.outerHTML")
    r.check("le mot de passe n'apparait nulle part dans la page", secret not in html)
finally:
    # Nettoyage : la camera de test (et son path MediaMTX) sont supprimes ---------------------------------------------------
    deleted = m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/sf/cameras/list', {method:'POST'}).then(x => x.json()).then(async list => {"
        " let n = 0; for (const c of list) { if (c.ip === '127.0.0.1') {"
        "  await fetch('/sf/cameras/delete', {method:'POST', headers:{'Content-Type':'application/x-www-form-urlencoded'}, body:'id=' + c.id}); n++; } }"
        " cb(n); });"
    )
    r.check("camera de test supprimee", deleted == 1, str(deleted))

r.finish()
