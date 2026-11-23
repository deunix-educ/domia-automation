#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page /admin/assistant (R12) avec un VRAI modele : reglages, test de connexion, une question de lecture, une action
proposee en carte puis confirmee, une autre annulee, la liste des modifications, « Nouvelle conversation ».

Non inclus dans la liste par defaut de tools/e2e.sh (il faut un llama-server) :
    AGENT_E2E_LLM_URL=http://127.0.0.1:8081 AGENT_E2E_MODEL=Qwen3-8B-Q4_K_M tools/e2e.sh assistant

Usage : python3 tests/e2e/assistant.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable), AGENT_E2E_LLM_URL, AGENT_E2E_MODEL.
"""
import os
import subprocess
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: assistant.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
llm = os.environ.get("AGENT_E2E_LLM_URL")
model = os.environ.get("AGENT_E2E_MODEL", "Qwen3-8B-Q4_K_M")
if not db_shell or not llm:
    sys.exit("E2E_DB_SHELL et AGENT_E2E_LLM_URL requis (voir l'en-tete)")

m = Marionette()
r = Report()


def sql(text):
    subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def enabled(selector):
    return m.wait_for(f"document.querySelector({selector!r}) && !document.querySelector({selector!r}).disabled", 20)


def saved(key):
    return m.wait_for(f"(document.querySelector('#setting-status-{key}')?.textContent || '').includes('Enregistr')", 8)


def ask(text, regime="daily"):
    """Envoie une demande ; attend la fin du tour (plus d'indicateur « reflechit »)."""
    m.js(f"document.querySelector('#assistant-regime').value = {regime!r}; document.querySelector('#assistant-regime').dispatchEvent(new Event('change', {{bubbles: true}}))")
    fill("#assistant-input", text)
    enabled("#assistant-send")
    m.click("#assistant-send")
    m.wait_for("document.querySelector('#assistant-busy') !== null", 10)
    return m.wait_for("document.querySelector('#assistant-busy') === null", 150, 0.5)


def log_text():
    return m.js("return document.querySelector('#assistant-log')?.innerText || ''")


# A minimal house: one shutter.
sql("INSERT INTO device (friendly_name, base_topic, device_type) VALUES ('Volet-salon', 'e2eassistant', 'cover');")

m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# Off by default ---------------------------------------------------------------------------------------------------
m.goto(url + "admin/assistant")
r.check("page rendue", m.wait_for("document.querySelector('#assistant-form') !== null", 20))
r.check("desactive par defaut : message affiche", m.wait_for("document.querySelector('#assistant-off') !== null", 8))
r.check("lien du menu d'administration actif",
        m.js("return [...document.querySelectorAll('#vc-nav-admin a')].some(a => a.getAttribute('href') === '/admin/assistant' && a.getAttribute('aria-current') === 'page')") is True)
r.check("envoi impossible tant que desactive", m.js("return document.querySelector('#assistant-send').disabled") is True)

# Settings -------------------------------------------------------------------------------------------------------------
m.goto(url + "admin/settings")
m.wait_for("document.querySelector('#setting-AGENT_ENABLED') !== null", 15)
for key, value in (("AGENT_LLM_URL", llm), ("AGENT_MODEL", model)):
    enabled(f"#setting-save-{key}")
    fill(f"#setting-{key}", value)
    m.click(f"#setting-save-{key}")
    r.check(f"{key} enregistre", saved(key))
enabled("#setting-save-AGENT_ENABLED")
m.click("#setting-AGENT_ENABLED")
m.click("#setting-save-AGENT_ENABLED")
r.check("AGENT_ENABLED enregistre", saved("AGENT_ENABLED"))

# Active page, connection test ---------------------------------------------------------------------------------------
m.goto(url + "admin/assistant")
m.wait_for("document.querySelector('#assistant-form') !== null", 20)
r.check("plus de message « desactive »", m.wait_for("document.querySelector('#assistant-off') === null", 8))
enabled("#assistant-test")
m.click("#assistant-test")
r.check("test de connexion : reponse", m.wait_for("(document.querySelector('#assistant-test-result')?.textContent || '').length > 3", 60),
        m.text("#assistant-test-result"))

# Reading --------------------------------------------------------------------------------------------------------------
done = ask("Quels volets y a-t-il dans la maison ?")
r.check("la lecture se termine", done)
r.check("la question apparait", "volets" in log_text().lower())
r.check("le volet du salon est cite dans la reponse", m.wait_for("document.querySelector('.vc-chat__reply') !== null", 5)
        and "salon" in m.js("return [...document.querySelectorAll('.vc-chat__reply')].map(e => e.textContent).join(' ')").lower(), log_text()[-300:])
r.check("aucune carte pour une simple lecture", m.js("return document.querySelector('.vc-chat__card') === null"))

# Action: a card, never executed without a click -------------------------------------------------------------------------
done = ask("Ferme le volet du salon")
r.check("le tour d'action se termine", done)
r.check("une carte est proposee", m.wait_for("document.querySelector('.vc-chat__card') !== null", 10), log_text()[-300:])
sentence = m.text(".vc-chat__card .vc-chat__changes-lines") or ""
r.check("la carte dit l'action en francais courant", sentence.strip() == "Fermer : Volet-salon", sentence)
r.check("le technique est replie dans les details",
        m.js("return document.querySelector('.vc-chat__card details.vc-chat__tech')?.open === false") is True)
r.check("rien n'est execute avant le clic", "Fait" not in log_text())
m.click(".assistant-confirm")
r.check("Confirmer : la carte devient « Fait »", m.wait_for("document.querySelector('.vc-chat__done') !== null", 30), log_text()[-300:])
r.check("la carte a disparu apres confirmation", m.wait_for("document.querySelector('.assistant-confirm') === null", 5))

# Cancelling another card ---------------------------------------------------------------------------------------------------
done = ask("Ouvre le volet du salon")
r.check("deuxieme action : carte", m.wait_for("document.querySelector('.assistant-cancel') !== null", 10), log_text()[-300:])
m.click(".assistant-cancel")
r.check("Annuler : action abandonnee", m.wait_for("document.querySelector('.assistant-cancel') === null", 10))
r.check("pas de seconde execution", log_text().count("Fait") <= 1, log_text()[-300:])

# List of the changes -------------------------------------------------------------------------------------------------------
m.goto(url + "admin/assistant")
m.wait_for("document.querySelector('#assistant-changes') !== null", 20)
r.check("la modification confirmee est listee, sans annulation possible (commande d'appareil)",
        m.wait_for("document.querySelector('#assistant-changes li') !== null", 10) and m.js("return document.querySelector('.assistant-undo') === null"),
        m.text("#assistant-changes"))

# New conversation -----------------------------------------------------------------------------------------------------------
m.click("#assistant-reset")
r.check("Nouvelle conversation vide le journal", m.wait_for("document.querySelectorAll('#assistant-log li').length === 0", 10))

r.finish()
