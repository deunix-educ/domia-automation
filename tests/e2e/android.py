#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des pages Android dans Firefox (R5 lot 4) avec un telephone SIMULE (tools/sim_android.py : memes trames que l'application
Kotlin, memes trames de controle qu'OkHttp) : appareil ajoute depuis l'interface (champ invalide refuse, jeton montre une fois),
presence, SMS et appels recus en direct, SMS ecrit dans le navigateur (en attente puis envoye / echoue en direct, refuse hors ligne),
jeton regenere (l'ancien est coupe en 4001), suppression.

Usage : python3 tests/e2e/android.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable). Aucun contact exterieur : le telephone ne parle qu'au serveur du test.
"""
import os
import subprocess
import sys
import time

here = __file__.rsplit("/", 1)[0]
sys.path.insert(0, here)
sys.path.insert(0, here + "/../../tools")
from marionette import Marionette, Report  # noqa: E402
from sim_android import Phone  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: android.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def ready(selector):
    return m.wait_for(f"document.querySelector({selector!r}) && !document.querySelector({selector!r}).disabled", 15)


def live():
    return m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# --- ajout depuis l'interface -------------------------------------------------------------------------------------------------------------------
m.goto(url + "android/devices")
r.check("page des appareils", m.wait_for("document.querySelector('#android-add-form') !== null", 10))
live()
r.check("aucun appareil au depart", m.wait_for("document.querySelector('#android-empty') !== null", 10))
ready("#android-add-submit")
m.click("#android-add-submit")
r.check("nom vide : erreur sur le champ", m.wait_for("document.querySelector('#crud-error-name') !== null", 10), m.text("#crud-error-name"))
fill("#android-name", "E2E Pixel")
fill("#android-phone", "abc")
m.click("#android-add-submit")
r.check("numero invalide : erreur sur le champ", m.wait_for("document.querySelector('#crud-error-phone_number') !== null", 10))
fill("#android-phone", "+33612345678")
m.click("#android-add-submit")
r.check("appareil enregistre : jeton affiche", m.wait_for("document.querySelector('#android-token-value')?.textContent.length === 36", 10))
token = m.text("#android-token-value").strip()
r.check("... le tableau liste l'appareil, hors ligne",
        m.wait_for("document.querySelector('#android-devices-table tbody tr[data-online=false]') !== null", 10))
r.check("... en base, le jeton n'est pas en clair", scalar("SELECT count(*) FROM android_device WHERE token_hash = '" + token + "'") == "0"
        and scalar("SELECT length(token_hash) FROM android_device") == "64")
m.click("#android-token-hide")
r.check("jeton masque a la demande", m.wait_for("document.querySelector('#android-token') === null", 5))
m.goto(url + "android/devices")
m.wait_for("document.querySelector('#android-devices-table') !== null", 10)
r.check("... et jamais reaffiche apres rechargement", token not in m.js("return document.body.innerText") and m.js("return document.querySelector('#android-token') === null"))

# --- presence en direct ------------------------------------------------------------------------------------------------------------------------------
m.goto(url + "android")
r.check("tableau de bord", m.wait_for("document.querySelector('#android-unread') !== null", 10))
live()
r.check("appareil hors ligne", m.wait_for("document.querySelector('#android-devices-table tbody tr .vc-badge[data-online=false]') !== null", 10))
phone = Phone(url, token, ping_interval=2)
phone.connect()
r.check("le serveur repond auth_ok", phone.wait_frame(lambda f: f.get("type") == "auth_ok" and f.get("device_name") == "E2E Pixel") is not None)
r.check("en ligne, sans recharger la page", m.wait_for("document.querySelector('#android-devices-table tbody tr .vc-badge[data-online=true]') !== null", 15))
r.check("... et en base", scalar("SELECT is_online FROM android_device") == "t")

# --- SMS recus : tableau de bord puis page ---------------------------------------------------------------------------------------------------------
phone.sms("+33699999999", "Salut depuis le telephone", contact="Alice")
r.check("SMS recu : non lu en direct", m.wait_for("document.querySelector('#android-unread')?.dataset.unread === '1'", 15), m.text("#android-unread"))
r.check("... dans les derniers SMS", m.wait_for("document.querySelector('#android-recent-sms li')?.textContent.includes('Salut depuis le telephone')", 10))
m.goto(url + "android/sms")
r.check("page des SMS", m.wait_for("document.querySelector('#android-sms-list') !== null", 10))
live()
r.check("le SMS recu est liste", m.wait_for("document.querySelector('#android-sms-list li[data-direction=incoming]')?.textContent.includes('Salut depuis le telephone')", 10))
r.check("... marque lu a l'affichage (comme la reference)", scalar("SELECT is_read FROM sms_message WHERE direction = 'incoming'") == "t")
phone.sms("+33688888888", "Second SMS en direct")
r.check("autre SMS : ajoute en direct", m.wait_for("document.querySelectorAll('#android-sms-list li[data-direction=incoming]').length === 2", 15))

# --- SMS ecrit dans le navigateur ------------------------------------------------------------------------------------------------------------------------
phone.ack = None  # l'application ne repond pas encore : l'envoi reste « en attente »
ready("#android-send")
fill("#android-to", "x")
m.click("#android-send")
r.check("destinataire et message invalides : erreurs sur les champs",
        m.wait_for("document.querySelector('#crud-error-to') !== null && document.querySelector('#crud-error-body') !== null", 10))
fill("#android-to", "+33611111111")
fill("#android-body", "Message du navigateur")
m.js("const b = document.querySelector('#android-body'); b.dispatchEvent(new Event('input', {bubbles: true}));")
m.click("#android-send")
order = phone.wait_frame(lambda f: f.get("type") == "send_sms")
r.check("le telephone recoit l'ordre d'envoi", order is not None and order["to"] == "+33611111111" and order["body"] == "Message du navigateur", str(order))
r.check("... en attente de l'appareil dans la page", m.wait_for("document.querySelector('#android-sms-list li[data-direction=outgoing][data-delivery=pending]') !== null", 15))
r.check("... avis de transmission", m.wait_for("document.querySelector('#android-notice[data-notice=queued]') !== null", 10))
phone.sms_ack(order["message_id"], True)
r.check("accuse de l'appareil : « envoye » en direct", m.wait_for("document.querySelector('#android-sms-list li[data-direction=outgoing][data-delivery=sent]') !== null", 15))
phone.ack = "fail"
fill("#android-to", "+33622222222")
fill("#android-body", "Deuxieme message")
m.click("#android-send")
r.check("accuse d'echec : « echec » et sa raison en direct",
        m.wait_for("[...document.querySelectorAll('#android-sms-list li[data-delivery=failed]')].some(l => l.textContent.includes('pas de reseau'))", 15))
r.check("... la file d'envoi garde les deux etats", scalar("SELECT string_agg(status, ',' ORDER BY id) FROM sms_outbox") == "sent,failed")

# --- appels --------------------------------------------------------------------------------------------------------------------------------------------------------------
m.goto(url + "android/calls")
r.check("journal d'appels vide", m.wait_for("document.querySelector('#android-calls-empty') !== null", 10))
live()
phone.call("ringing", "0612", contact="Bob")
r.check("appel entrant : ligne en cours, en direct", m.wait_for("document.querySelector('#android-calls-table tbody tr[data-open=true]') !== null", 15))
phone.call("ended", "0612", contact="Bob", duration=125)
r.check("appel termine : duree 02:05", m.wait_for("document.querySelector('#android-calls-table tbody tr[data-open=false]')?.textContent.includes('02:05')", 15))
phone.call("ringing", "0699")
phone.call("missed", "0699")
r.check("appel manque", m.wait_for("document.querySelector('#android-calls-table tbody tr[data-direction=missed]') !== null", 15))
r.check("... deux appels au journal, un seul ouvert jamais", scalar("SELECT count(*) FROM call_log") == "2" and scalar("SELECT count(*) FROM call_log WHERE ended_at IS NULL") == "0")

# --- hors ligne : l'envoi est refuse -----------------------------------------------------------------------------------------------------------------------
phone.close()
m.goto(url + "android/sms")
m.wait_for("document.querySelector('#android-presence') !== null", 10)
r.check("telephone parti : hors ligne en direct", m.wait_for("document.querySelector('#android-presence[data-online=false]') !== null", 15))
ready("#android-send")
fill("#android-to", "+33633333333")
fill("#android-body", "Personne ne repond")
m.click("#android-send")
r.check("envoi refuse : « l'appareil n'est pas connecte »", m.wait_for("document.querySelector('#android-notice[data-notice=offline]') !== null", 10))
r.check("... rien n'est enregistre", scalar("SELECT count(*) FROM sms_message WHERE direction = 'outgoing'") == "2")

# --- nouveau jeton : l'ancien est coupe ----------------------------------------------------------------------------------------------------------------------
m.goto(url + "android/devices")
m.wait_for("document.querySelector('.android-regenerate') !== null", 10)
ready(".android-regenerate")
m.click(".android-regenerate")
r.check("nouveau jeton affiche", m.wait_for("document.querySelector('#android-token-value')?.textContent.length === 36", 10))
fresh = m.text("#android-token-value").strip()
r.check("... different de l'ancien", fresh != token)
old = Phone(url, token)
old.connect()
r.check("l'ancien jeton est refuse (fermeture 4001)", old.wait_closed(10) and old.close_code == 4001, str(old.close_code))
new = Phone(url, fresh)
new.connect()
r.check("le nouveau jeton marche", new.wait_frame(lambda f: f.get("type") == "auth_ok") is not None)
new.close()

# --- suppression ------------------------------------------------------------------------------------------------------------------------------------------------------
m.goto(url + "android/devices")
m.wait_for("document.querySelector('.android-delete') !== null", 10)
ready(".android-delete")
m.click(".android-delete")
r.check("suppression : confirmation demandee", m.wait_for("document.querySelector('.android-delete-confirm') !== null", 10))
m.click(".android-delete-confirm")
r.check("device deleted", m.wait_for("document.querySelector('#android-empty') !== null", 15))
r.check("... avec ses SMS et appels", scalar("SELECT (SELECT count(*) FROM sms_message) + (SELECT count(*) FROM call_log) + (SELECT count(*) FROM android_device)") == "0")
sys.exit(r.finish())
