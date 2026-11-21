#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des jauges systeme de la barre laterale dans Firefox : rendues par le serveur (vides), remplies par le
WebSocket sans recharger la page, largeur des barres appliquee malgre la CSP, valeurs coherentes avec
`/api/system-stats/`, et flux `system` effectivement continu cote serveur.

Usage : python3 tests/e2e/system.py url utilisateur_admin mot_de_passe
Le serveur doit mesurer toutes les secondes (SYSTEM_STATS_SECS=1, positionne par tools/e2e.sh).
"""
import json
import re
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: system.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]

m = Marionette()
r = Report()


def fetch_json(path):
    return json.loads(
        m.js_async(
            "const cb = arguments[arguments.length-1];"
            f"fetch({path!r}).then(async x => cb(JSON.stringify({{status: x.status, body: await x.json().catch(() => null)}})));"
        )
    )


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
anon = fetch_json("/api/system-stats/")
r.check("anonyme : /api/system-stats/ refuse (401)", anon["status"] == 401, str(anon["status"]))
login = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", login == 200)

# --- page rendue : les jauges existent, la mesure arrive par le WebSocket -------------------------------------------
m.goto(url)
r.check("les trois jauges sont dans la barre laterale",
        m.wait_for("['cpu','mem','disk'].every(k => document.querySelector('#vc-stat-' + k + '-bar') && document.querySelector('#vc-stat-' + k + '-pct'))", 20))
m.js("window.__no_reload = 42")
r.check("la jauge processeur se remplit toute seule (« … » -> « N% »)",
        m.wait_for("/^\\d+%$/.test(document.querySelector('#vc-stat-cpu-pct').textContent.trim())", 20),
        m.js("return document.querySelector('#vc-stat-cpu-pct').textContent"))
r.check("la jauge memoire est remplie",
        m.wait_for("/^\\d+%$/.test(document.querySelector('#vc-stat-mem-pct').textContent.trim())", 10))
r.check("sans recharger la page", m.js("return window.__no_reload") == 42)

# --- barre : largeur appliquee par la CSSOM malgre la CSP ---------------------------------------------------------
w = m.js(
    "const b = document.querySelector('#vc-stat-mem-bar'); const t = b.parentElement;"
    "return JSON.stringify({bar: b.getBoundingClientRect().width, track: t.getBoundingClientRect().width, attr: b.getAttribute('style')})"
)
w = json.loads(w)
r.check("la barre memoire a une largeur (CSSOM autorisee par la CSP)", w["bar"] > 0 and w["bar"] <= w["track"] + 1, str(w))

# --- coherence avec l'API et infobulles --------------------------------------------------------------------------
api = fetch_json("/api/system-stats/")
r.check("API : 200 avec les cles du contrat",
        api["status"] == 200 and sorted(api["body"]) == ["cpu", "disk", "memory", "temperature", "updated_at"], str(api))
dom_mem = int(m.js("return parseInt(document.querySelector('#vc-stat-mem-pct').textContent)"))
r.check("memoire : jauge et API concordent (a 3 points pres)", abs(dom_mem - api["body"]["memory"]["percent"]) <= 3,
        f"jauge {dom_mem} api {api['body']['memory']['percent']}")
tip = m.js("return document.querySelector('#vc-stat-mem').title")
r.check("infobulle memoire : « Mémoire : x Go / y Go (p %) »", re.match(r"^Mémoire : [\d.]+ Go / [\d.]+ Go \([\d.]+ %\)$", tip) is not None, tip)
tip = m.js("return document.querySelector('#vc-stat-cpu').title")
r.check("infobulle processeur : nombre de coeurs", re.match(r"^Processeur : [\d.]+ % \(\d+ cœurs\)$", tip) is not None, tip)
temp = m.js("return document.querySelector('#vc-stat-temp-value')?.textContent || null")
r.check("temperature : absente ou « N °C »", temp is None or re.match(r"^\d+ °C$", temp) is not None, str(temp))

# --- le serveur pousse bien en continu ---------------------------------------------------------------------------
count = m.js_async(
    "const cb = arguments[arguments.length-1]; let n = 0, last = 0, seen = [];"
    "const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws/notifications/');"
    "ws.onmessage = (e) => { const d = JSON.parse(e.data); if (d.type === 'hello') ws.send(JSON.stringify({type:'subscribe', topic:'system'}));"
    "  if (d.type === 'system_stats') { n++; seen.push(d.updated_at); } };"
    "setTimeout(() => { ws.close(); cb(JSON.stringify({n, distinct: new Set(seen).size})); }, 4500);"
)
c = json.loads(count)
r.check("flux `system` : au moins 3 mesures distinctes en 4,5 s (1 s d'intervalle)", c["n"] >= 3 and c["distinct"] >= 3, str(c))

# --- anglais : les infobulles suivent la langue ------------------------------------------------------------------
m.js("document.cookie = 'lang=en; path=/'; localStorage.setItem('lang', 'en')")
m.goto(url)
m.wait_for("/^\\d+%$/.test(document.querySelector('#vc-stat-mem-pct')?.textContent.trim())", 20)
tip = m.js("return document.querySelector('#vc-stat-mem').title")
r.check("anglais : infobulle « Memory: x GB / y GB »", re.match(r"^Memory: [\d.]+ GB / [\d.]+ GB \([\d.]+%\)$", tip) is not None, tip)
m.js("document.cookie = 'lang=fr; path=/'; localStorage.setItem('lang', 'fr')")

r.finish()
