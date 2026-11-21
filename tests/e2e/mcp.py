#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page /admin/mcp (R11 lot M1, ADR 0059) : creation d'un jeton, valeur montree UNE fois, appel reel de /mcp avec ce
jeton, validation du formulaire, revocation avec confirmation puis refus immediat, journal d'audit, traduction a chaud.

Usage : python3 tests/e2e/mcp.py url utilisateur_admin mot_de_passe
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
    sys.exit("usage: mcp.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
tag = f"E2E-MCP-{int(time.time())}"

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def call_mcp(token, body=None):
    """Une requete JSON-RPC sur /mcp ; renvoie (statut, corps)."""
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    payload = json.dumps(body or {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).encode()
    req = urllib.request.Request(url + "mcp", data=payload, method="POST", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def row_state(name):
    return m.js(
        "const tr = [...document.querySelectorAll('#mcp-tokens-table tbody tr')].find(t => t.querySelector('td').textContent === "
        f"{name!r}); return tr ? tr.dataset.state : null;"
    )


def click_in_row(name, selector):
    return m.js(
        "const tr = [...document.querySelectorAll('#mcp-tokens-table tbody tr')].find(t => t.querySelector('td').textContent === "
        f"{name!r}); if (!tr) return false; const b = tr.querySelector({selector!r}); if (!b || b.disabled) return false;"
        "b.click(); return true;"
    )


# Etat propre, puis connexion par l'API.
m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# Sans jeton, personne n'entre ------------------------------------------------------------------------------------
status, _ = call_mcp(None)
r.check("/mcp sans jeton : 401", status == 401, str(status))
status, _ = call_mcp("dnm_" + "x" * 43)
r.check("/mcp avec un jeton inconnu : 401", status == 401, str(status))

m.goto(url + "admin/mcp")
r.check("page rendue", m.wait_for("document.querySelector('#mcp-create-form') !== null", 15))
r.check("serveur actif annonce", "/mcp" in (m.text("#mcp-status") or ""), m.text("#mcp-status"))
r.check("lien du menu d'administration actif",
        m.js("return [...document.querySelectorAll('#vc-nav-admin a')].some(a => a.getAttribute('href') === '/admin/mcp' && a.getAttribute('aria-current') === 'page')") is True)

# Validation cote serveur ------------------------------------------------------------------------------------------
m.wait_for("document.querySelector('#mcp-create-submit') && !document.querySelector('#mcp-create-submit').disabled", 20)
fill("#mcp-name", "   ")
m.click("#mcp-create-submit")
r.check("nom vide : erreur sous le champ",
        m.wait_for("(document.querySelector('#crud-error-name')?.textContent || '').includes('obligatoire')", 8),
        m.text("#crud-error-name"))
m.js("document.querySelector('#mcp-scope-lecture').click()")  # decoche la seule portee
fill("#mcp-name", tag)
m.click("#mcp-create-submit")
r.check("aucune portee : erreur sous le champ", m.wait_for("document.querySelector('#crud-error-scopes') !== null", 8))
r.check("aucun jeton montre apres un refus", m.js("return document.querySelector('#mcp-token') === null"))

# Creation : la valeur n'est montree qu'une fois ------------------------------------------------------------------------
m.js("document.querySelector('#mcp-scope-lecture').click()")  # recoche
m.js("document.querySelector('#mcp-scope-pilotage').click()")
m.click("#mcp-create-submit")
r.check("jeton cree : valeur affichee", m.wait_for("document.querySelector('#mcp-token-value') !== null", 10))
value = m.text("#mcp-token-value") or ""
command = m.text("#mcp-token-command") or ""
r.check("forme de la valeur : dnm_ + 43 caracteres", re.fullmatch(r"dnm_[A-Za-z0-9_-]{43}", value) is not None, value)
r.check("la commande Claude Code contient le jeton et /mcp",
        value in command and "claude mcp add" in command and "/mcp" in command, command)

# Le jeton marche vraiment -----------------------------------------------------------------------------------------
status, body = call_mcp(value)
r.check("/mcp avec le jeton : 200", status == 200, f"{status} {body[:120]}")
r.check("l'outil system_status est propose", "system_status" in body, body[:200])
status, body = call_mcp(value, {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                                "params": {"name": "system_status", "arguments": {}}})
r.check("system_status repond (portee lecture)", status == 200 and '"database":"ok"' in body.replace(" ", ""), body[:200])

# Une fois masque ou la page rechargee, la valeur a disparu ------------------------------------------------------------
m.click("#mcp-token-hide")
r.check("masquer retire la valeur", m.wait_for("document.querySelector('#mcp-token') === null", 5))
m.goto(url + "admin/mcp")
m.wait_for("document.querySelector('#mcp-tokens-table') !== null", 15)
r.check("apres rechargement, la valeur n'est plus nulle part",
        m.js(f"return !document.documentElement.innerHTML.includes({value!r});") is True)
r.check("le jeton est liste et actif", row_state(tag) == "active", str(row_state(tag)))
r.check("seul le debut de la valeur est montre",
        m.js(f"return [...document.querySelectorAll('#mcp-tokens-table tbody tr')].some(t => t.textContent.includes({tag!r}) && t.textContent.includes({value[:12]!r}) && !t.textContent.includes({value[:20]!r}));") is True)

# Revocation avec confirmation, puis refus immediat ----------------------------------------------------------------
m.wait_for("document.querySelector('.mcp-revoke') && !document.querySelector('.mcp-revoke').disabled", 20)
r.check("clic sur Revoquer", click_in_row(tag, ".mcp-revoke"))
r.check("confirmation demandee", m.wait_for("document.querySelector('.mcp-revoke-confirm') !== null", 5))
status, _ = call_mcp(value)
r.check("pas encore revoque tant qu'on n'a pas confirme", status == 200, str(status))
m.js("document.querySelector('.mcp-revoke-confirm').click()")
r.check("jeton revoque dans la liste", m.wait_for(
    "[...document.querySelectorAll('#mcp-tokens-table tbody tr')].some(t => t.querySelector('td').textContent === "
    f"{tag!r} && t.dataset.state === 'revoked')", 8))
status, _ = call_mcp(value)
r.check("/mcp avec le jeton revoque : 401 des l'appel suivant", status == 401, str(status))

# Journal d'audit -------------------------------------------------------------------------------------------------------
r.check("journal : creation et revocation", m.wait_for(
    "(() => { const t = document.querySelector('#mcp-audit-table'); return t && t.innerText.includes('Jeton créé') && t.innerText.includes('Jeton révoqué'); })()", 8))
r.check("journal : l'auteur est l'administrateur",
        m.js(f"return [...document.querySelectorAll('#mcp-audit-table tbody tr')].some(t => t.children[1].textContent === {user!r});") is True)
r.check("journal : aucune valeur de jeton", m.js(f"return !document.querySelector('#mcp-audit').innerHTML.includes({value!r});") is True)
m.js("const s = document.querySelector('#mcp-audit-kind'); s.value = 'denied'; s.dispatchEvent(new Event('change', {bubbles: true}));")
r.check("filtre sur les refus : rien d'enregistre", m.wait_for("document.querySelector('#mcp-audit-empty') !== null", 8))
m.js("const s = document.querySelector('#mcp-audit-kind'); s.value = ''; s.dispatchEvent(new Event('change', {bubbles: true}));")

# Traduction a chaud ---------------------------------------------------------------------------------------------------
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("formulaire et etats en anglais", m.wait_for(
    "document.querySelector('#mcp-create-submit').textContent === 'Create the token' && "
    "[...document.querySelectorAll('#mcp-tokens-table tbody tr')].some(t => t.dataset.state === 'revoked' && t.textContent.includes('revoked'))", 8))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")
m.wait_for("document.querySelector('#mcp-create-submit').textContent === 'Créer le jeton'", 8)

r.finish()
