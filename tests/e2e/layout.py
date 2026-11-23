#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la mise en page : barre laterale rendue par le serveur, lien actif, repli memorise, langue, deconnexion.

Usage : python3 tests/e2e/layout.py [url] [utilisateur] [mot_de_passe]
Prerequis : serveur demarre, base amorcee (`dominet seed`), compte existant, Firefox en mode Marionette.
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

url = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:3000/"
user = sys.argv[2] if len(sys.argv) > 2 else "admin"
password = sys.argv[3] if len(sys.argv) > 3 else None
if not password:
    sys.exit("mot de passe requis (3e argument)")

m = Marionette()
r = Report()
m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
m.goto(url)

r.check("anonyme : pas de barre laterale", m.js("return document.querySelector('.vc-sidebar')") is None)

res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion par l'API", res == 200, str(res))
m.goto(url)

# Server render: the sidebar is in the initial HTML (before any hydration).
r.check("barre laterale presente", m.wait_for("document.querySelector('.vc-sidebar') !== null", 8))
r.check("contenu decale par le CSS (marge = largeur de la barre)",
        m.js("return parseInt(getComputedStyle(document.querySelector('.vc-main-wrap')).marginLeft)") > 100)
r.check("nom d'utilisateur affiche", user in (m.text("#vc-user-name") or ""), m.text("#vc-user-name"))
top = m.js("return [...document.querySelectorAll('#vc-nav-top .vc-nav-link span:last-child')].map(e => e.textContent)")
r.check("navigation du haut (4 entrees)", len(top) == 4 and "Surveillance" in top, str(top))
r.check("icones SVG inline", m.js("return document.querySelectorAll('.vc-sidebar svg').length") >= 8)
r.check("lien actif : Tableau de bord",
        m.js("return document.querySelector('.vc-sidebar-nav a[aria-current=page]')?.textContent.trim()") == "Tableau de bord")

# Wait for hydration by clicking on the collapse until it is taken into account.
collapsed = False
for _ in range(40):
    m.click(".vc-sidebar-collapse-btn")
    if m.wait_for("document.querySelector('.vc-sidebar').classList.contains('vc-sidebar--collapsed')", 0.5):
        collapsed = True
        break
r.check("repli de la barre laterale", collapsed)
# The content has a .25 s transition on margin-left: we wait for the animation to end.
r.check("contenu pleine largeur apres repli",
        m.wait_for("parseInt(getComputedStyle(document.querySelector('.vc-main-wrap')).marginLeft) === 0", 5))
r.check("repli memorise (localStorage)", m.js("return localStorage.getItem('vc-sidebar-collapsed')") == "1")
m.goto(url)
r.check("repli restaure apres rechargement",
        m.wait_for("document.querySelector('.vc-sidebar').classList.contains('vc-sidebar--collapsed')", 10))
m.click(".vc-sidebar-open-btn")
r.check("reouverture", m.wait_for("!document.querySelector('.vc-sidebar').classList.contains('vc-sidebar--collapsed')", 5))
r.check("reouverture memorisee", m.js("return localStorage.getItem('vc-sidebar-collapsed')") == "0")

# Live language: the bar's labels change without reloading.
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("libelles en anglais", m.wait_for("document.querySelector('.vc-logout-btn').textContent.includes('Sign out')", 8))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")
r.check("libelles en francais", m.wait_for("document.querySelector('.vc-logout-btn').textContent.includes('Déconnexion')", 8))

# Unknown page: a real 404, but still within the layout.
m.goto(url + "page-inexistante")
r.check("404 dans la mise en page", m.wait_for("document.querySelector('.vc-sidebar') !== null", 8))

# Logout
m.goto(url)
done = False
for _ in range(40):
    m.click("#vc-logout")
    if m.wait_for("document.querySelector('.vc-sidebar') === null", 0.8):
        done = True
        break
r.check("deconnexion : la barre laterale disparait", done)
r.check("deconnexion : session fermee cote serveur",
        m.js_async("const cb = arguments[arguments.length-1]; fetch('/sf/whoami', {method:'POST'}).then(x => x.text()).then(cb);") == "null")

r.finish()
