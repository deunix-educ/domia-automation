#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Captures d'ecran des pages (pas un test : aucune verification). Sert a relire la mise en page.

Usage : SHOT_DIR=dossier tools/e2e.sh shots     (ou : python3 tests/e2e/shots.py url utilisateur_admin mot_de_passe)
Variables : SHOT_DIR (defaut /tmp/dominet-shots), SHOT_WIDTH (defaut 1280), SHOT_PAGES (liste separee par des virgules, defaut ci-dessous).
"""
import base64
import os
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: shots.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
out = os.environ.get("SHOT_DIR", "/tmp/dominet-shots")
width = int(os.environ.get("SHOT_WIDTH", "1280"))
pages = os.environ.get(
    "SHOT_PAGES",
    "messaging/compose,admin/cameras!add,admin/locations!add,admin/settings,surveillance/discovery,account,admin/users!add,admin/alert-rules!add",
).split(",")
os.makedirs(out, exist_ok=True)

m = Marionette()
r = Report()
m.cmd("WebDriver:SetWindowRect", {"width": width, "height": 900})
m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
for spec in pages:
    path, _, action = spec.partition("!")
    m.goto(url + path)
    m.wait_for("document.querySelector('main') !== null", 10)
    if action == "add":  # opens the CRUD component's form
        for _ in range(40):
            m.click("#crud-add")
            if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
                break
    else:
        m.wait_for("false", 1.5)
    shot = m.cmd("WebDriver:TakeScreenshot", {"full": True, "hash": False})["value"]
    name = path.replace("/", "-") + ("-form" if action else "") + ".png"
    with open(f"{out}/{name}", "wb") as f:
        f.write(base64.b64decode(shot))
    r.check(f"capture {name}", True)
r.finish()
