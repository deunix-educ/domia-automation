#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page /account/cameras (ADR 0064 lot 4) : lien depuis « Mon compte », validation, creation d'un mot de passe de lecteur
(montre UNE fois, jamais relisible), acceptation par la route que MediaMTX interroge, revocation avec confirmation puis refus
immediat, traduction a chaud.

Usage : python3 tests/e2e/camera_access.py url utilisateur mot_de_passe
"""
import json
import re
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: camera_access.py url utilisateur mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
tag = f"E2E-ACCES-{int(time.time())}"

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def ask_route(name, secret):
    """What MediaMTX asks DomiNet before a read: status of the decision (200 allowed, 401 refused)."""
    body = {"action": "read", "user": name, "password": secret, "token": secret, "ip": "127.0.0.1", "path": "cam", "protocol": "rtsp"}
    req = urllib.request.Request(url + "internal/mediamtx/auth", data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.status
    except urllib.error.HTTPError as e:
        return e.code


def row_state(name):
    return m.js(
        "const tr = [...document.querySelectorAll('#camaccess-table tbody tr')].find(t => t.querySelector('td').textContent === "
        f"{name!r}); return tr ? tr.dataset.state : null;"
    )


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion", res == 200, str(res))

# The page is reached from "My account" --------------------------------------------------------------------------------
m.goto(url + "account")
r.check("« Mon compte » propose le lien", m.wait_for("document.querySelector('#account-camera-access') !== null", 15))
m.js("document.querySelector('#account-camera-access').click()")
r.check("le lien ouvre la page d'acces aux cameras", m.wait_for("document.querySelector('#camaccess-create-form') !== null", 15))
r.check("l'URL est /account/cameras", m.js("return location.pathname") == "/account/cameras")
r.check("le nom d'utilisateur a donner au lecteur est celui du compte",
        m.wait_for(f"document.querySelector('#camaccess-username')?.textContent === {user!r}", 10), str(m.text("#camaccess-username")))

# Validation -------------------------------------------------------------------------------------------------------------
m.wait_for("document.querySelector('#camaccess-create-submit') && !document.querySelector('#camaccess-create-submit').disabled", 20)
fill("#camaccess-name", "   ")
m.click("#camaccess-create-submit")
r.check("nom vide : erreur sous le champ",
        m.wait_for("(document.querySelector('#crud-error-name')?.textContent || '').includes('obligatoire')", 8), m.text("#crud-error-name"))
r.check("aucun mot de passe montre apres un refus", m.js("return document.querySelector('#camaccess-secret') === null"))

# Creation: the password is only shown once --------------------------------------------------------------------------
fill("#camaccess-name", tag)
m.click("#camaccess-create-submit")
r.check("mot de passe cree et affiche", m.wait_for("document.querySelector('#camaccess-secret-password') !== null", 10))
secret = m.text("#camaccess-secret-password") or ""
shown_user = m.text("#camaccess-secret-user") or ""
r.check("forme du mot de passe : dns_ + 43 caracteres", re.fullmatch(r"dns_[A-Za-z0-9_-]{43}", secret) is not None, secret)
r.check("le nom d'utilisateur affiche est celui du compte", shown_user == user, shown_user)

# The route MediaMTX asks accepts it, with the account's name only ---------------------------------------------------------
r.check("la route d'authentification accepte (nom du compte + mot de passe)", ask_route(user, secret) == 200)
r.check("... sans tenir compte de la casse du nom", ask_route(user.upper(), secret) == 200)
r.check("... mais pas un autre nom", ask_route("quelquun-d-autre", secret) == 401)
r.check("... ni un mauvais mot de passe", ask_route(user, secret[:-1] + ("A" if secret[-1] != "A" else "B")) == 401)

# Once hidden or the page reloaded, the password is gone -----------------------------------------------------------------
m.click("#camaccess-hide")
r.check("masquer retire le mot de passe", m.wait_for("document.querySelector('#camaccess-secret') === null", 5))
m.goto(url + "account/cameras")
m.wait_for("document.querySelector('#camaccess-table') !== null", 15)
r.check("apres rechargement, le mot de passe n'est plus nulle part",
        m.js(f"return !document.documentElement.innerHTML.includes({secret!r});") is True)
r.check("le mot de passe est liste et actif", row_state(tag) == "active", str(row_state(tag)))
r.check("seul le debut est montre",
        m.js(f"return [...document.querySelectorAll('#camaccess-table tbody tr')].some(t => t.textContent.includes({tag!r}) && t.textContent.includes({secret[:12]!r}) && !t.textContent.includes({secret!r}));") is True)
r.check("la derniere utilisation est renseignee apres l'usage",
        m.js(f"return [...document.querySelectorAll('#camaccess-table tbody tr')].some(t => t.children[0].textContent === {tag!r} && !t.children[3].textContent.includes('jamais'));") is True)

# Revocation with confirmation, then immediate refusal -----------------------------------------------------------------------
m.wait_for("document.querySelector('.camaccess-revoke') && !document.querySelector('.camaccess-revoke').disabled", 20)
m.js("[...document.querySelectorAll('#camaccess-table tbody tr')].find(t => t.children[0].textContent === " + repr(tag) + ").querySelector('.camaccess-revoke').click()")
r.check("confirmation demandee", m.wait_for("document.querySelector('.camaccess-revoke-confirm') !== null", 5))
r.check("pas encore revoque tant qu'on n'a pas confirme", ask_route(user, secret) == 200)
m.js("document.querySelector('.camaccess-revoke-confirm').click()")
r.check("mot de passe revoque dans la liste", m.wait_for(
    "[...document.querySelectorAll('#camaccess-table tbody tr')].some(t => t.children[0].textContent === " + repr(tag) + " && t.dataset.state === 'revoked')", 8))
r.check("un message dit ce qui a ete ferme", m.wait_for("document.querySelector('#camaccess-revoked') !== null", 8), str(m.text("#camaccess-revoked")))
r.check("la route refuse des l'appel suivant", ask_route(user, secret) == 401)

# Live translation -----------------------------------------------------------------------------------------------------------
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("formulaire et etats en anglais", m.wait_for(
    "document.querySelector('#camaccess-create-submit').textContent === 'Create the password' && "
    "[...document.querySelectorAll('#camaccess-table tbody tr')].some(t => t.dataset.state === 'revoked' && t.textContent.includes('revoked'))", 8))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")
m.wait_for("document.querySelector('#camaccess-create-submit').textContent === 'Créer le mot de passe'", 8)

r.finish()
