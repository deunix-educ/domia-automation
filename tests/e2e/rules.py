#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des regles d'alerte Prometheus et Grafana (R7 lot 2) : les regles livrees par `dominet seed` sont la, une regle s'ajoute,
se valide cote serveur (duree invalide), se modifie, se supprime, et chaque changement reecrit le fichier de provisionnement
(PROMETHEUS_RULES_PATH / GRAFANA_ALERTING_PATH du serveur d'essai, sans adresse de service : rechargement « a faire a la main »).

Usage : python3 tests/e2e/rules.py url utilisateur_admin mot_de_passe
"""
import os
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: rules.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
tag = f"E2E{int(time.time())}"
prom_file = os.environ.get("E2E_PROMETHEUS_RULES")
graf_file = os.environ.get("E2E_GRAFANA_RULES")

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def rows():
    return m.js("return [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent)") or []


def wait_rows(predicate_js, timeout=10):
    return m.wait_for(
        "(() => { const rows = [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent);"
        f" return {predicate_js}; }})()",
        timeout,
    )


def save_enabled():
    return m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)


def click_in_row(name, selector):
    return m.js(
        "const tr = [...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === "
        f"{name!r}); if (!tr) return false; tr.querySelector({selector!r}).click(); return true;"
    )


def wait_file(path, predicate, timeout=10):
    """Le fichier est reecrit apres la reponse du serveur : on attend, on ne lit pas tout de suite."""
    end = time.time() + timeout
    while time.time() < end:
        if predicate(read(path)):
            return True
        time.sleep(0.2)
    return False


def read(path):
    try:
        return open(path, encoding="utf-8").read() if path else ""
    except OSError:
        return ""


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))


def open_form():
    for _ in range(40):  # until the interface is hydrated, the click opens nothing
        m.click("#crud-add")
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


# ------------------------------------------------------------------------------------------------------------- Prometheus
m.goto(url + "admin/prometheus-rules")
r.check("page Prometheus rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("les regles livrees sont la", wait_rows("rows.includes('DominetDown') && rows.includes('MediaMTXDown')"), str(rows()))
if prom_file:
    r.check("rules.yml ecrit au demarrage", "alert: \"DominetDown\"" in read(prom_file), read(prom_file)[:120])

name = f"{tag}Rule"
r.check("le formulaire s'ouvre", open_form())
save_enabled()
fill("#crud-field-name", name)
fill("#crud-field-expr", "vector(1) > 0")
fill("#crud-field-summary", "Resume de test")
fill("#crud-field-for_duration", "bientot")
m.click("#crud-save")
r.check("duree invalide : erreur sous le champ", m.wait_for("document.querySelector('#crud-error-for_duration') !== null", 8))
r.check("le formulaire reste ouvert", m.js("return document.querySelector('#crud-form') !== null"))
fill("#crud-field-for_duration", "2m")
m.click("#crud-save")
r.check("ajout : ligne dans le tableau", wait_rows(f"rows.includes({name!r})"), str(rows()))
if prom_file:
    r.check("rules.yml contient la nouvelle regle", wait_file(prom_file, lambda t: f"alert: \"{name}\"" in t))

click_in_row(name, ".crud-edit")
m.wait_for(f"document.querySelector('#crud-field-name')?.value === {name!r}", 5)
save_enabled()
m.js("document.querySelector('#crud-field-active').click()")
m.click("#crud-save")
r.check("regle coupee : plus dans rules.yml", not prom_file or wait_file(prom_file, lambda t: f"alert: \"{name}\"" not in t and "DominetDown" in t))

m.click("#rules-sync-prometheus-run")
r.check("bouton : fichier ecrit, rechargement a faire a la main",
        m.wait_for("(document.querySelector('#rules-sync-prometheus-result')?.textContent || '').includes('Fichier écrit')", 8))

click_in_row(name, ".crud-delete")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression", wait_rows(f"!rows.includes({name!r})"), str(rows()))

# ----------------------------------------------------------------------------------------------------------------- Grafana
m.goto(url + "admin/grafana-rules")
r.check("page Grafana rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("les regles livrees sont la", wait_rows("rows.some(t => t.startsWith('Heat wave')) && rows.includes('MediaMTX inaccessible')"), str(rows()))
if graf_file:
    text = read(graf_file)
    r.check("rules.yaml ecrit au demarrage", "apiVersion: 1" in text and "meteo-canicule" in text, text[:120])

title = f"{tag} Regle Grafana"
r.check("le formulaire s'ouvre", open_form())
save_enabled()
fill("#crud-field-title", title)
fill("#crud-field-expr", "vector(1) > 0")
fill("#crud-field-summary", "Resume")
# The browser already bounds the numeric field: the server's check is seen through the API.
bad = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/sf/grafana-rules/save', {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({input:"
    "{id:null, uid:'', title:'x', folder:'F', group:'G', group_interval:'1m', datasource_uid:'prometheus', expr:'up', query_window:0,"
    "for_duration:'5m', severity:'warning', summary:'s', description:'', no_data_state:'NoData', exec_err_state:'Error', active:true}})})"
    ".then(x => x.text()).then(cb);"
)
r.check("fenetre hors bornes : refusee par le serveur", "Invalid" in str(bad) and "query_window" in str(bad), str(bad)[:150])
fill("#crud-field-query_window", "120")
m.click("#crud-save")
r.check("ajout : ligne dans le tableau", wait_rows(f"rows.includes({title!r})"), str(rows()))
uid = f"{tag.lower()}-regle-grafana"
r.check("l'identifiant est tire du titre", m.wait_for(f"document.querySelector('#crud-table').textContent.includes({uid!r})", 8),
        str(m.js("return document.querySelector('#crud-table').textContent.slice(0, 200)")))
if graf_file:
    r.check("rules.yaml contient la nouvelle regle", wait_file(graf_file, lambda t: f"uid: \"{uid}\"" in t))

click_in_row(title, ".crud-delete")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression", wait_rows(f"!rows.includes({title!r})"), str(rows()))
if graf_file:
    r.check("rules.yaml ne la contient plus", wait_file(graf_file, lambda t: uid not in t and "meteo-canicule" in t))

# A non-administrator does not have these pages (they are guarded like the other administration pages).
r.finish()
