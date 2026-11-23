#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page d'accueil (compte sans tableau) : rendu serveur, hydratation WASM, traductions, session.

Usage : python3 tests/e2e/home.py url utilisateur mot_de_passe
Le serveur doit tourner et Firefox etre lance en mode Marionette (voir marionette.py).
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
# Clean state: neither cookie (session, language) nor local storage from a previous run.
m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
m.goto(url)

# The root is protected: an anonymous visitor is sent to the login page.
m.goto(url)
r.check("anonyme : renvoye vers /login", m.js("return location.pathname") == "/login", m.js("return location.pathname"))
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion par l'API depuis le navigateur", res == 200, str(res))
m.goto(url)

# Server render
r.check("langue par defaut : francais", m.js("return document.documentElement.lang") == "fr")
r.check("page rendue (titre)", m.js("return document.title") == "DomiNet", m.js("return document.title"))
r.check("texte francais present", "Aucun tableau" in (m.text("#vc-dash-empty h2") or ""), m.text("#vc-dash-empty h2"))
r.check("session vue par le rendu serveur", user in (m.text("#vc-user-name") or ""), m.text("#vc-user-name"))

# Hydration: the menu collapse only moves if the WASM has taken over
r.check("WASM charge", m.wait_for("performance.getEntriesByType('resource').some(e => e.name.endsWith('.wasm'))", 15))
# The WASM file is loaded before being instantiated: we click until the first click is taken into account
# (= hydration is finished), then we check the toggle in both directions.
collapsed = "document.querySelector('.vc-sidebar')?.classList.contains('vc-sidebar--collapsed')"
hydrated = False
for _ in range(40):
    m.click(".vc-sidebar-collapse-btn")
    if m.wait_for(collapsed, 0.5):
        hydrated = True
        break
r.check("hydratation terminee (1er clic pris en compte)", hydrated)
m.click(".vc-sidebar-open-btn")
r.check("hydratation : le menu se deplie", m.wait_for("!" + collapsed, 8))

# Live translations, without reloading
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
ok = m.wait_for("document.documentElement.lang === 'en' && document.querySelector('#vc-dash-empty h2').textContent.includes('No dashboard')", 8)
r.check("bascule EN a chaud", ok, m.text("#vc-dash-empty h2"))
r.check("cookie de langue ecrit", "lang=en" in m.js("return document.cookie"), m.js("return document.cookie"))
m.goto(url)
r.check("langue conservee apres rechargement (rendu serveur)", m.js("return document.documentElement.lang") == "en")
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent))?.click()")

r.finish()
