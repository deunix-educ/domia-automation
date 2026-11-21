#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la messagerie dans Firefox (R5 lot 5) : l'administrateur compose (champs invalides refuses), l'envoi est asynchrone et la page
de detail suit les statuts en direct (e-mails vers un SMTP SIMULE, SMS confie a un telephone SIMULE qui accuse a son rythme), le
destinataire retrouve le message dans sa boite de reception sans le tableau des autres, un compte ordinaire n'a pas la page d'ecriture,
et sans telephone en ligne les SMS echouent avec leur raison.

Usage : python3 tests/e2e/messaging.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable), E2E_SMTP_FILE (messages ecrits par tools/sim-smtp.py). Aucun contact exterieur.
"""
import json
import os
import subprocess
import sys

here = __file__.rsplit("/", 1)[0]
sys.path.insert(0, here)
sys.path.insert(0, here + "/../../tools")
from marionette import Marionette, Report  # noqa: E402
from sim_android import Phone  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: messaging.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell, smtp_file = os.environ.get("E2E_DB_SHELL"), os.environ.get("E2E_SMTP_FILE")
if not db_shell or not smtp_file:
    sys.exit("E2E_DB_SHELL / E2E_SMTP_FILE absents : lancer par tools/e2e.sh")

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def mails(subject="Coupure d"):
    """E-mails du SMTP simule portant ce sujet : le fichier est partage avec les autres scripts (alertes, automations), qui envoient aussi."""
    with open(smtp_file) as f:
        return [x for x in (json.loads(line) for line in f if line.strip()) if f"Subject: {subject}" in x["data"]]


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def ready(selector):
    return m.wait_for(f"document.querySelector({selector!r}) && !document.querySelector({selector!r}).disabled", 15)


def live():
    return m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)


def login(name):
    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    res = m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{name!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )
    return res == 200


def wait_mails(n, timeout=15):
    import time
    end = time.time() + timeout
    while time.time() < end:
        if len(mails()) >= n:
            return True
        time.sleep(0.3)
    return False


def row(recipient):
    """Etat (e-mail, SMS) d'un destinataire dans le tableau de la page de detail."""
    return m.js(
        "const tr = [...document.querySelectorAll('#messaging-deliveries tbody tr')].find(t => t.children[0].textContent.trim() === "
        f"{recipient!r}); return tr ? tr.dataset.email + '/' + tr.dataset.sms : null;"
    )


def wait_row(recipient, expected, timeout=15):
    return m.wait_for(
        "(() => { const tr = [...document.querySelectorAll('#messaging-deliveries tbody tr')].find(t => t.children[0].textContent.trim() === "
        f"{recipient!r}); return !!tr && tr.dataset.email + '/' + tr.dataset.sms === {expected!r}; }})()", timeout)


m.goto(url)
m.js("localStorage.clear()")
r.check("connexion administrateur", login(admin))

# --- donnees : deux destinataires (meme mot de passe que l'administrateur), un telephone ------------------------------------------------------------
sql(
    "UPDATE users SET email = 'root@example.test' WHERE username = " + repr(admin) + ";"
    "INSERT INTO users (username, email, mobile, password_hash, role) SELECT 'alice', 'alice@example.test', '+33611111111', password_hash, 'user' FROM users WHERE username = "
    + repr(admin) + ";"
    "INSERT INTO users (username, email, mobile, password_hash, role) SELECT 'bob', 'bob@example.test', '', password_hash, 'user' FROM users WHERE username = "
    + repr(admin) + ";"
)
m.goto(url + "android/devices")
ready("#android-add-submit")
fill("#android-name", "E2E Pixel")
fill("#android-phone", "+33600000000")
m.click("#android-add-submit")
m.wait_for("document.querySelector('#android-token-value')?.textContent.length === 36", 10)
token = m.text("#android-token-value").strip()
phone = Phone(url, token, ping_interval=2, ack=None)  # le telephone n'accuse pas tout seul : on le fait a la main
phone.connect()
r.check("telephone simule connecte", phone.wait_frame(lambda f: f.get("type") == "auth_ok") is not None)

# --- composer : champs invalides ---------------------------------------------------------------------------------------------------------------------------
m.goto(url + "messaging/compose")
r.check("page d'ecriture", m.wait_for("document.querySelector('#messaging-form') !== null", 10))
live()
r.check("... les comptes actifs sont proposes", m.wait_for("document.querySelector('#messaging-user-alice') !== null && document.querySelector('#messaging-user-bob') !== null", 10))
r.check("... bob est signale sans mobile", "no mobile" in m.js("return document.querySelector('#messaging-user-bob').closest('label').textContent"))
ready("#messaging-send")
m.click("#messaging-send")
r.check("tout vide : sujet, message et destinataires refuses",
        m.wait_for("['subject','body','recipients'].every(f => document.querySelector('#crud-error-' + f) !== null)", 10))
r.check("... rien n'est enregistre", scalar("SELECT count(*) FROM message") == "0")

# --- envoi asynchrone, suivi en direct -------------------------------------------------------------------------------------------------------------------
fill("#messaging-subject", "Coupure d'eau")
fill("#messaging-body", "Demain de 9h a 12h.")
m.click("#messaging-channel-both")
m.click("#messaging-user-alice")
m.click("#messaging-user-bob")
m.click("#messaging-send")
r.check("envoi : on arrive sur la page du message", m.wait_for("/\\/messaging\\/\\d+$/.test(location.pathname)", 15), m.js("return location.pathname"))
r.check("... avec son tableau de livraison", m.wait_for("document.querySelector('#messaging-deliveries') !== null", 15))
live()
order = phone.wait_frame(lambda f: f.get("type") == "send_sms")
r.check("le telephone recoit l'ordre d'envoi pour alice", order is not None and order["to"] == "+33611111111" and order["body"] == "Demain de 9h a 12h.", str(order))
r.check("e-mails partis vers alice et bob (SMTP simule)", m.wait_for("true", 1) and wait_mails(2) and sorted(x["to"][0] for x in mails()) == ["alice@example.test", "bob@example.test"], str(mails()))
r.check("... avec le sujet et le texte", len(mails()) == 2 and all("Demain de 9h a 12h." in x["data"] for x in mails()))
r.check("alice : e-mail envoye, SMS en attente de l'accuse (en direct)", wait_row("alice", "sent/pending"), str(row("alice")))
r.check("bob : e-mail envoye, SMS sans objet (pas de mobile)", wait_row("bob", "sent/na"), str(row("bob")))
r.check("le message est « en cours » tant que le SMS attend", m.js("return document.querySelector('#messaging-message').dataset.status") == "sending")
phone.sms_ack(order["message_id"], True)
r.check("accuse du telephone : SMS « envoye » en direct", wait_row("alice", "sent/sent"))
r.check("... et le message passe a « termine »", m.wait_for("document.querySelector('#messaging-message')?.dataset.status === 'sent'", 15))
msg_id = scalar("SELECT id FROM message")

# --- liste des messages envoyes --------------------------------------------------------------------------------------------------------------------------------
m.goto(url + "messaging")
r.check("liste des envoyes : le message y est", m.wait_for("document.querySelector('#messaging-table tbody tr[data-status=sent]')?.textContent.includes('Coupure d')", 10))
m.goto(url + "messaging/")
r.check("/messaging/ avec barre finale", m.wait_for("document.querySelector('#messaging-table') !== null", 10))

# --- le destinataire ----------------------------------------------------------------------------------------------------------------------------------------------------
r.check("connexion d'alice", login("alice"))
m.goto(url + "messaging/inbox")
r.check("boite de reception : le message est la", m.wait_for("document.querySelector('#messaging-table tbody tr')?.textContent.includes('Coupure d')", 10))
m.click("#messaging-table tbody tr a")
r.check("detail : le texte est visible", m.wait_for("document.querySelector('#messaging-text')?.textContent.includes('Demain de 9h')", 10))
r.check("... son propre etat, sans le tableau des autres", m.wait_for("document.querySelector('#messaging-mine') !== null", 10)
        and m.js("return document.querySelector('#messaging-deliveries') === null"))
m.goto(url + "messaging/compose")
r.check("alice n'a pas la page d'ecriture", m.wait_for("document.querySelector('#messaging-form') === null && document.body.innerText.length > 0", 10))
r.check("... ni celle des messages envoyes", (m.goto(url + "messaging"), m.wait_for("document.querySelector('#messaging-table') === null", 10))[1])

# --- sans telephone : les SMS echouent avec leur raison --------------------------------------------------------------------------------------------
phone.close()
r.check("connexion administrateur (2)", login(admin))
m.goto(url + "android")
m.wait_for("document.querySelector('#android-devices-table') !== null", 10)
r.check("telephone parti : hors ligne en base", m.wait_for("true", 1) and scalar("SELECT is_online FROM android_device") in ("f", ""), scalar("SELECT is_online FROM android_device"))
m.goto(url + "messaging/compose")
ready("#messaging-send")
fill("#messaging-subject", "Second")
fill("#messaging-body", "Sans telephone")
m.click("#messaging-channel-sms")
m.click("#messaging-user-alice")
m.click("#messaging-send")
r.check("SMS sans appareil en ligne : echec par destinataire", m.wait_for("/\\/messaging\\/\\d+$/.test(location.pathname)", 15) and wait_row("alice", "na/failed"), str(row("alice")))
r.check("... avec la raison", "no Android device" in m.js("return document.querySelector('#messaging-deliveries tbody').innerText"))
r.check("... et le message est termine", m.wait_for("document.querySelector('#messaging-message')?.dataset.status === 'sent'", 15))

# --- message inconnu -------------------------------------------------------------------------------------------------------------------------------------------------------
m.goto(url + "messaging/999999")
r.check("message inconnu : introuvable", m.wait_for("document.querySelector('#messaging-not-found') !== null", 10))
sys.exit(r.finish())
