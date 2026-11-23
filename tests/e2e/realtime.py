#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E du temps reel et de la CSP dans Firefox : la CSP est appliquee, le WebSocket se connecte malgre elle, un message
diffuse arrive a l'ecran (texte seulement), la deconnexion ferme la connexion, et la connexion se retablit toute seule
apres un redemarrage du serveur.

Usage : python3 tests/e2e/realtime.py url utilisateur_admin mot_de_passe [commande_de_redemarrage]
La commande de redemarrage (facultative) doit redemarrer le serveur et ne rendre la main qu'une fois qu'il repond.
"""
import os
import re
import subprocess
import sys
import time
import urllib.parse

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: realtime.py url utilisateur_admin mot_de_passe [commande_de_redemarrage]")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
restart_cmd = sys.argv[4] if len(sys.argv) > 4 else None

m = Marionette()
r = Report()

# The simulator only acts on devices that exist: we create two (prepared in SQL on the throw-away database).
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")


def new_device(name):
    out = subprocess.run(db_shell, shell=True, input=f"INSERT INTO device (friendly_name) VALUES ('{name}') RETURNING id;".encode(),
                         check=True, capture_output=True).stdout.decode()
    return int(re.search(r"^\s*(\d+)\s*$", out, re.M).group(1))


dev_a, dev_b = new_device("E2E Sim A"), new_device("E2E Sim B")
dev_unknown = max(dev_a, dev_b) + 1000  # number without a device: used to check that nothing is followed any more


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def rt_state():
    return m.js("return document.querySelector('#vc-rt-state')?.dataset.state || null")


def wait_state(state, timeout=15):
    return m.wait_for(f"document.querySelector('#vc-rt-state')?.dataset.state === {state!r}", timeout)


def api_login():
    return m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
r.check("connexion administrateur", api_login() == 200)

# --- CSP, headers, compression ----------------------------------------------------------------------------------------


def diagnose_hang(cookie, what):
    """Page qui ne repond plus : le serveur est-il vivant, et la meme page repond-elle a un autre client ?"""
    import urllib.request

    def probe(path, with_cookie):
        t = time.time()
        try:
            req = urllib.request.Request(url.rstrip("/") + path, headers={"Cookie": cookie} if with_cookie else {})
            with urllib.request.urlopen(req, timeout=10) as x:
                x.read()
                return f"{x.status} en {time.time() - t:.1f}s"
        except Exception as e:  # noqa: BLE001
            return f"{e!r} apres {time.time() - t:.1f}s"

    print(f"DIAGNOSTIC {what} : /healthz {probe('/healthz', False)} ; /admin/settings {probe('/admin/settings', True)} ; "
          f"/ {probe('/', True)}", flush=True)


cookie = "; ".join(f"{c['name']}={c['value']}" for c in m.cmd("WebDriver:GetCookies"))
try:
    m.goto(url + "admin/settings")
except TimeoutError:
    diagnose_hang(cookie, "goto admin/settings")
    raise
r.check("page rendue et hydratee (bouton actif)", m.wait_for("document.querySelector('#broadcast-send') && !document.querySelector('#broadcast-send').disabled", 20))
headers = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch(location.href).then(x => cb(JSON.stringify({csp: x.headers.get('content-security-policy'), cc: x.headers.get('cache-control'),"
    " coop: x.headers.get('cross-origin-opener-policy')}))).catch(e => cb(JSON.stringify({error: String(e)})));"
)
import json  # noqa: E402

h = json.loads(headers)
if "error" in h:  # network failure: we say so instead of waiting for the script's timeout
    sys.exit(f"fetch(location.href) a echoue : {h['error']} (url {m.js('return location.href')})")
r.check("CSP presente avec un nonce", "'nonce-" in (h["csp"] or ""), str(h["csp"])[:120])
r.check("pages jamais mises en cache", h["cc"] == "no-store", str(h["cc"]))
r.check("isolation (COOP)", h["coop"] == "same-origin", str(h["coop"]))
asset = json.loads(m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/pkg/dominet.wasm').then(x => cb(JSON.stringify({cc: x.headers.get('cache-control'), enc: x.headers.get('content-encoding'),"
    " type: x.headers.get('content-type')})));"
))
r.check("WASM : revalidation (no-cache)", asset["cc"] == "no-cache", str(asset))
r.check("WASM compresse", asset["enc"] == "gzip", str(asset))
r.check("WASM : type MIME application/wasm", (asset["type"] or "").startswith("application/wasm"), str(asset))

# An inline script without a nonce, injected into the page, must be blocked by the CSP.
blocked = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "let violated = false;"
    "document.addEventListener('securitypolicyviolation', () => { violated = true; });"
    "window.__csp_bypass = undefined;"
    "const s = document.createElement('script'); s.textContent = 'window.__csp_bypass = 1';"
    "document.body.appendChild(s);"
    "setTimeout(() => cb(JSON.stringify({ran: window.wrappedJSObject ? window.wrappedJSObject.__csp_bypass === 1 : window.__csp_bypass === 1, violated})), 300);"
)
b = json.loads(blocked)
r.check("script en ligne sans nonce : bloque par la CSP", b["ran"] is False and b["violated"] is True, blocked)

# --- WebSocket ---------------------------------------------------------------------------------------------------------
r.check("temps reel : connecte (le WebSocket passe la CSP)", wait_state("online"), str(rt_state()))
r.check("indicateur lisible", "connecté" in (m.text("#vc-rt-state") or ""), m.text("#vc-rt-state"))

# --- Broadcast: the message arrives, as text only -----------------------------------------------------------------------------
payload = "Alerte <b onmouseover=alert(1)>test</b> & co"
fill("#broadcast-text", payload)
m.click("#broadcast-send")
r.check("diffusion acceptee", m.wait_for("(document.querySelector('#broadcast-status')?.textContent || '').includes('envoyé')", 8), m.text("#broadcast-status"))
r.check("notification affichee a l'ecran", m.wait_for("document.querySelector('.vc-toast .vc-toast-text') !== null", 8))
r.check("texte identique, rien n'est interprete comme du HTML",
        m.js("return document.querySelector('.vc-toast .vc-toast-text').textContent") == payload
        and m.js("return document.querySelector('.vc-toast .vc-toast-text b')") is None)
m.js("document.querySelector('.vc-toast-close').click()")
r.check("notification fermee a la main", m.wait_for("document.querySelector('.vc-toast') === null", 5))
fill("#broadcast-text", "Message qui s'efface seul")
m.click("#broadcast-send")
r.check("notification temporaire apparait", m.wait_for("document.querySelector('.vc-toast') !== null", 8))
r.check("... et disparait toute seule", m.wait_for("document.querySelector('.vc-toast') === null", 14))

# --- Subjects: subscription by device number, retained state, invalid states --------------------------------------------------
def inject(device, state, available=True):
    body = f"device_id={device}&state={urllib.parse.quote(state)}&available={'true' if available else 'false'}"
    return m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/sf/devices/inject', {method:'POST', headers:{'Content-Type':'application/x-www-form-urlencoded'},"
        f"body: {body!r}}}).then(x => x.text()).then(cb);"
    )


def received():
    return m.js("return document.querySelector('#sim-received')?.textContent.trim() || ''")


r.check("carte d'essai : aucun etat recu au depart", "Aucun état reçu" in received(), received())
enabled_sim = "document.querySelector('#sim-send') && !document.querySelector('#sim-send').disabled"
m.wait_for(enabled_sim, 15)
fill("#sim-device", str(dev_a))
m.click("#sim-send")
r.check("injection d'un etat : diffusee a l'onglet abonne", m.wait_for("(document.querySelector('#sim-status')?.textContent || '').includes('Diffusé à 1')", 8), m.text("#sim-status"))
r.check("l'etat arrive en direct dans la page", m.wait_for("document.querySelector('#sim-received').textContent.includes('E2E Sim A')", 8), received())
r.check("... avec ses proprietes", '"temperature":21.5' in received(), received())

# A state injected BEFORE the subscription is delivered upon subscription (last state retained on the server side).
inject(dev_b, '{"state":"OFF","battery":88}', False)
fill("#sim-device", str(dev_b))
r.check("nouvel abonne : recoit le dernier etat retenu", m.wait_for("document.querySelector('#sim-received').textContent.includes('battery')", 8), received())
r.check("... y compris l'indisponibilite", "false" in received().split("|")[1] if "|" in received() else False, received())

# Changing the number changes the subject: the old state is no longer displayed, and nothing is received for it any more.
fill("#sim-device", str(dev_unknown))
r.check("autre appareil sans etat : message vide", m.wait_for("document.querySelector('#sim-received').textContent.includes('Aucun état reçu')", 5), received())
inject(dev_b, '{"state":"ON","battery":87}')
time.sleep(0.8)
r.check("un appareil abandonne n'est plus suivi", "Aucun état reçu" in received(), received())

# Server-side validation.
fill("#sim-device", str(dev_a))
fill("#sim-state", "[1, 2]")
m.click("#sim-send")
r.check("etat non objet : refuse, message traduit", m.wait_for("(document.querySelector('#sim-status')?.textContent || '').includes('objet JSON')", 8), m.text("#sim-status"))
# Firefox itself blocks the submission of a 0 (min attribute); we remove it to exercise the interface's message.
m.js("document.querySelector('#sim-device').removeAttribute('min')")
fill("#sim-device", "0")
m.click("#sim-send")
r.check("numero invalide : refuse", m.wait_for("(document.querySelector('#sim-status')?.textContent || '').includes('appareil existant')", 8), m.text("#sim-status"))
fill("#sim-state", '{"state": "ON", "temperature": 21.5}')
fill("#sim-device", str(dev_a))

# A valid number but without a device is refused too (the simulator does not invent a device).
fill("#sim-device", str(dev_unknown))
m.click("#sim-send")
r.check("appareil inexistant : refuse", m.wait_for("(document.querySelector('#sim-status')?.textContent || '').includes('appareil existant')", 8), m.text("#sim-status"))
fill("#sim-device", str(dev_a))

# --- Switching language: the indicator follows ------------------------------------------------------------------------------
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("indicateur traduit", m.wait_for("(document.querySelector('#vc-rt-state')?.textContent || '').includes('connected')", 5), m.text("#vc-rt-state"))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")

# --- Automatic reconnection after a server restart ----------------------------------------------------------------------------
if restart_cmd:
    subprocess.run(restart_cmd, shell=True, check=True)
    r.check("apres redemarrage : la connexion est retablie toute seule", wait_state("online", 40), str(rt_state()))
    fill("#broadcast-text", "Apres redemarrage")
    m.click("#broadcast-send")
    r.check("... et les messages arrivent a nouveau", m.wait_for("document.querySelector('.vc-toast') !== null", 10))
    m.js("document.querySelector('.vc-toast-close')?.click()")
    # The server forgot everything (subscriptions and retained states): the browser must ask AGAIN for the followed device (7).
    inject(dev_a, '{"state":"APRES-REDEMARRAGE"}')
    r.check("apres redemarrage : l'appareil suivi est de nouveau abonne",
            m.wait_for("document.querySelector('#sim-received').textContent.includes('APRES-REDEMARRAGE')", 10), received())
else:
    print("SKIP  reconnexion apres redemarrage (pas de commande fournie)")

# --- Logout: no more sidebar, no more connection ------------------------------------------------------------------------------
done = False
for _ in range(30):
    m.click("#vc-logout")
    if m.wait_for("location.pathname === '/login' && document.querySelector('#vc-rt-state') === null", 1.5):
        done = True
        break
r.check("deconnexion : l'indicateur disparait", done)
r.check("deconnecte : le WebSocket est refuse (401)",
        m.js_async(
            "const cb = arguments[arguments.length-1];"
            "const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws/notifications/');"
            "ws.onopen = () => cb('open'); ws.onerror = () => cb('refused'); setTimeout(() => cb('timeout'), 5000);"
        ) == "refused")
r.check("reconnexion : le temps reel repart", api_login() == 200 and (m.goto(url) or True) and wait_state("online", 20), str(rt_state()))

r.finish()
