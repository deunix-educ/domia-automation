#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des alertes de la plateforme dans Firefox (R13 lot E2), contre un Prometheus SIMULE (`tools/sim-prometheus.py`, lance par
`tools/e2e.sh` quand ce script est demande) : section « Plateforme » de /alerts en lecture seule, pastille de la barre laterale,
alerte qui disparait quand sa cause disparait, source injoignable dite (et non confondue avec « aucune alerte »), section absente
des filtres « resolues » et « acquittees ».

Usage : tools/e2e.sh --image dominet:dev platform_alerts
"""
import os
import sys
import time
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: platform_alerts.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
mode_url = os.environ.get("E2E_PROM_MODE_URL")
if not mode_url:
    sys.exit("E2E_PROM_MODE_URL absent : lancer par tools/e2e.sh platform_alerts")

m = Marionette()
r = Report()


def set_mode(mode):
    urllib.request.urlopen(f"{mode_url}/{mode}", timeout=5).read()


def badge():
    """Texte de la pastille (None si elle est cachee)."""
    return m.js("const b = document.querySelector('#vc-alert-badge'); return b && !b.hidden ? b.textContent.trim() : null")


def wait_until(predicate, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.3)
    return False


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# --- an ongoing alert at Prometheus ------------------------------------------------------------------------------------------------------
set_mode("firing")
time.sleep(3)  # the read happens every 2 s in this run
m.goto(url + "alerts")
r.check("page des alertes", m.wait_for("document.querySelector('#alerts-filters') !== null", 10))
r.check("section Plateforme presente", m.wait_for("document.querySelector('#platform-alerts') !== null", 10))
r.check("l'alerte en cours est listee avec sa source et sa gravite",
        m.wait_for("[...document.querySelectorAll('#platform-table tbody tr')].some(t => t.textContent.includes('SimMqttDown') && t.dataset.source === 'prometheus' && t.dataset.severity === 'critical')", 10),
        m.text("#platform-alerts"))
r.check("... le resume est affiche", "MQTT simule hors ligne" in m.text("#platform-table"))
r.check("... l'alerte en attente n'y est pas", "SimLent" not in m.text("#platform-alerts"))
r.check("... lecture seule : aucun bouton dans la section", m.js("return document.querySelector('#platform-alerts button') === null"))
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
r.check("la pastille compte l'alerte de la plateforme", wait_until(lambda: badge() == "1"), str(badge()))
r.check("aucune alerte d'appareil : le tableau des appareils est absent", m.js("return document.querySelector('#alerts-table') === null"))

# --- the "resolved" and "acknowledged" filters do not show the platform ---------------------------------------------------------------------
m.goto(url + "alerts?status=resolved")
r.check("filtre resolues : page chargee", m.wait_for("document.querySelector('#alerts-filters') !== null && document.querySelector('#alerts-empty, #alerts-table') !== null", 10))
r.check("... section Plateforme absente", m.js("return document.querySelector('#platform-alerts') === null"))

# --- the cause disappears: the alert disappears, live ----------------------------------------------------------------------------------------------
m.goto(url + "alerts")
m.wait_for("document.querySelector('#platform-table') !== null", 10)
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
set_mode("empty")
r.check("alerte disparue de la liste sans recharger la page", m.wait_for("document.querySelector('#platform-empty') !== null", 15), m.text("#platform-alerts"))
r.check("... la pastille se cache", wait_until(lambda: badge() is None), str(badge()))

# --- Prometheus unreachable: said, never confused with "no alert" --------------------------------------------------------------------------------
set_mode("down")
time.sleep(3)
m.goto(url + "alerts")
r.check("source injoignable annoncee", m.wait_for("document.querySelector('#platform-down-prometheus') !== null", 10), m.text("#platform-alerts"))
r.check("... sans « aucune alerte de la plateforme » trompeur", m.js("return document.querySelector('#platform-empty') === null"))
r.check("... ni message vide general", m.js("return document.querySelector('#alerts-empty')?.hidden !== false"))

# --- back: the alert reappears --------------------------------------------------------------------------------------------------------------------------------
set_mode("firing")
time.sleep(3)
m.goto(url + "alerts")
r.check("l'alerte reapparait quand Prometheus revient", m.wait_for("[...document.querySelectorAll('#platform-table tbody tr')].some(t => t.textContent.includes('SimMqttDown'))", 10))
r.check("... le message d'injoignabilite disparait", m.js("return document.querySelector('#platform-down-prometheus') === null"))
sys.exit(r.finish())
