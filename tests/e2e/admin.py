#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des pages /admin/navbar et /admin/settings : la barre laterale suit les modifications de navigation, les
reglages sont valides selon leur type, un secret n'est jamais affiche. Tout ce qui est modifie est retabli.

Usage : python3 tests/e2e/admin.py url utilisateur_admin mot_de_passe
Prerequis : base amorcee (`dominet seed`) et serveur lance avec DOMINET_MASTER_KEY.
"""
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: admin.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
tag = f"E2E{int(time.time())}"
secret = f"secret-{tag}"

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def select(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); e.value = {value!r};"
        "e.dispatchEvent(new Event('change', {bubbles: true})); return e.value;"
    )


def enabled(selector, timeout=15):
    return m.wait_for(f"document.querySelector({selector!r}) && !document.querySelector({selector!r}).disabled", timeout)


def labels():
    return m.js("return [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent)") or []


def sidebar():
    return m.js("return [...document.querySelectorAll('#vc-nav-top .vc-nav-link span:last-child, #vc-nav-bottom .vc-nav-link span:last-child')].map(e => e.textContent)") or []


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))

# ============================================== Navigation ======================================================
m.goto(url + "admin/navbar")
r.check("page Navigation rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("les 7 entrees amorcees sont listees", len(labels()) >= 7, str(labels()))
r.check("liens d'administration dans la barre",
        m.js("return [...document.querySelectorAll('#vc-nav-admin a')].map(a => a.getAttribute('href'))") ==
        ["/admin/locations", "/admin/cameras", "/admin/navbar", "/admin/settings", "/admin/users", "/admin/dashboards", "/admin/widgets",
         "/admin/mqtt-sources", "/admin/devices", "/admin/device-groups", "/admin/device-labels", "/admin/weather-stations",
         "/admin/scenes", "/admin/automations", "/admin/alert-rules", "/admin/prometheus-rules", "/admin/grafana-rules", "/admin/logs", "/admin/mcp", "/admin/assistant"])

# The content does not slide under the sidebar (sibling rule `.vc-sidebar + .vc-main-wrap`)
r.check("contenu decale a droite de la barre laterale",
        m.js("return document.querySelector('.vc-main-wrap').getBoundingClientRect().left >= document.querySelector('.vc-sidebar').getBoundingClientRect().width - 1") is True)

# --- Administration menu: hideable drawer on the right (the links stay in the DOM, hidden when it is closed) --------
def drawer():
    return m.js("const d = document.querySelector('#vc-admin-drawer'); return {hidden: d.getAttribute('aria-hidden'),"
                "visible: getComputedStyle(d).visibility, left: Math.round(d.getBoundingClientRect().left),"
                "vw: window.innerWidth, toggle: getComputedStyle(document.querySelector('#vc-admin-toggle')).opacity}")


closed = drawer()
r.check("tiroir d'administration ferme au chargement", closed["hidden"] == "true" and closed["visible"] == "hidden" and closed["left"] >= closed["vw"] - 1, str(closed))
r.check("languette visible sur le bord droit", closed["toggle"] == "1", str(closed))
for _ in range(40):  # until the interface is hydrated, the click has no effect
    m.js("document.querySelector('#vc-admin-toggle').click()")
    if m.wait_for("document.querySelector('#vc-admin-drawer').getAttribute('aria-hidden') === 'false'", 0.5):
        break
time.sleep(0.5)
opened = drawer()
r.check("clic sur la languette : le tiroir s'ouvre", opened["hidden"] == "false" and opened["visible"] == "visible" and opened["left"] < opened["vw"] - 200, str(opened))
r.check("liens visibles dans le tiroir ouvert", m.js("return document.querySelector('#vc-nav-admin a').offsetParent !== null"))
m.js("window.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape'}))")
time.sleep(0.5)
r.check("Echap ferme le tiroir", drawer()["hidden"] == "true")
m.js("document.querySelector('#vc-admin-toggle').click()")
r.check("tiroir rouvert", m.wait_for("document.querySelector('#vc-admin-drawer').getAttribute('aria-hidden') === 'false'", 5))
m.js("document.querySelector('#vc-admin-close').click()")
time.sleep(0.5)
r.check("bouton de fermeture du tiroir", drawer()["hidden"] == "true")

# --- Truncated cells: the tooltip (title) carries the full value ----------
r.check("chaque cellule de texte du tableau porte son title",
        m.js("const ok = [...document.querySelectorAll('#crud-table tbody tr')].every(tr => [...tr.querySelectorAll('td:not(:last-child)')]"
             ".every(td => td.querySelector('.vc-badge') || td.getAttribute('title') === td.textContent)); return ok;") is True)

# --- Editing from a long list: back to the top of the page -------
m.js("const d = document.createElement('div'); d.id = 'e2e-spacer'; d.style.height = '4000px'; document.querySelector('.vc-main').appendChild(d); window.scrollTo(0, 1500);")
r.check("page defilee avant le clic", m.wait_for("window.scrollY > 500", 5))  # scrolling is animated (scroll-behavior: smooth)
m.js("document.querySelector('.crud-edit').click()")
r.check("clic sur Modifier : retour au debut de la page", m.wait_for("window.scrollY === 0", 5))
m.js("document.querySelector('#e2e-spacer')?.remove()")

for _ in range(40):
    m.click("#crud-add")
    if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
        break
r.check("formulaire ouvert", m.js("return document.querySelector('#crud-form') !== null"))
r.check("liste Position : haut par defaut", m.js("return document.querySelector('#crud-field-position').value") == "top")
r.check("ordre 0 par defaut", m.js("return document.querySelector('#crud-field-sort_order').value") == "0")
r.check("administrateurs seulement : decoche par defaut", m.js("return document.querySelector('#crud-field-admin_only').checked") is False)
r.check("aides affichees sous les champs", m.js("return document.querySelectorAll('.vc-hint').length") >= 3)

# Server-side refusal: icon unknown to Lucide, dangerous URL.
enabled("#crud-save")
label = f"{tag}-Entree"
fill("#crud-field-label", label)
fill("#crud-field-icon_name", "no-such-icon-xyz")
fill("#crud-field-url", "javascript:alert(1)")
m.click("#crud-save")
r.check("icone inconnue : erreur sous le champ", m.wait_for("document.querySelector('#crud-error-icon_name') !== null", 8))
r.check("URL javascript: refusee", m.wait_for("document.querySelector('#crud-error-url') !== null", 8))
r.check("rien n'a ete ajoute", label not in labels())

fill("#crud-field-icon_name", "house")
fill("#crud-field-url", "/maison-e2e/")
fill("#crud-field-sort_order", "999")
enabled("#crud-save")
m.click("#crud-save")
r.check("ajout : ligne dans le tableau",
        m.wait_for(f"[...document.querySelectorAll('#crud-table tbody tr')].some(tr => tr.querySelector('td').textContent === {label!r})", 8))
r.check("la barre laterale suit, sans rechargement", m.wait_for(f"[...document.querySelectorAll('#vc-nav-top .vc-nav-link')].some(a => a.textContent.trim() === {label!r})", 8), str(sidebar()))
r.check("icone SVG de la nouvelle entree",
        m.js(f"const a = [...document.querySelectorAll('#vc-nav-top .vc-nav-link')].find(a => a.textContent.trim() === {label!r}); return !!a && !!a.querySelector('svg')") is True)

# Editing: goes to the bottom, reserved to administrators, new window.
m.js(f"[...document.querySelectorAll('#crud-table tbody tr')].find(tr => tr.querySelector('td').textContent === {label!r}).querySelector('.crud-edit').click()")
r.check("formulaire prerempli (liste deroulante comprise)",
        m.wait_for("document.querySelector('#crud-field-position')?.value === 'top' && document.querySelector('#crud-field-sort_order').value === '999'", 5))
enabled("#crud-save")
select("#crud-field-position", "bottom")
select("#crud-field-url_target", "_blank")
m.js("document.querySelector('#crud-field-admin_only').click()")
m.click("#crud-save")
r.check("deplacee en bas dans la barre", m.wait_for(f"[...document.querySelectorAll('#vc-nav-bottom .vc-nav-link')].some(a => a.textContent.trim() === {label!r} && a.getAttribute('target') === '_blank')", 8), str(sidebar()))
r.check("plus dans le haut", m.js(f"return ![...document.querySelectorAll('#vc-nav-top .vc-nav-link')].some(a => a.textContent.trim() === {label!r})") is True)

# Live translation of the drop-down lists and the table
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("tableau traduit (position Bottom)", m.wait_for(f"(() => {{ const tr = [...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === {label!r}); return !!tr && [...tr.querySelectorAll('td')].some(td => td.textContent === 'Bottom'); }})()", 8))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")
m.wait_for("document.querySelector('.vc-page-header h2').textContent === 'Navigation'", 8)

# Deletion
m.js(f"[...document.querySelectorAll('#crud-table tbody tr')].find(tr => tr.querySelector('td').textContent === {label!r}).querySelector('.crud-delete').click()")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression : plus dans le tableau", m.wait_for(f"![...document.querySelectorAll('#crud-table tbody tr')].some(tr => tr.querySelector('td').textContent === {label!r})", 8))
r.check("suppression : plus dans la barre", m.wait_for(f"![...document.querySelectorAll('.vc-sidebar .vc-nav-link')].some(a => a.textContent.trim() === {label!r})", 8))

# ============================================== Settings ========================================================
m.goto(url + "admin/settings")
r.check("page Reglages rendue", m.wait_for("document.querySelector('#setting-MOTION_THRESHOLD') !== null", 10))
r.check("17 reglages affiches", m.js("return document.querySelectorAll('.vc-setting').length") == 17)
original = m.js("return document.querySelector('#setting-MOTION_THRESHOLD').value")
r.check("type numerique pour un nombre", m.js("return document.querySelector('#setting-MOTION_THRESHOLD').type") == "number")
r.check("le secret est un champ mot de passe, vide",
        m.js("const e = document.querySelector('#setting-SMS_FREE_KEY'); return e.type + '|' + e.value") == "password|")

enabled("#setting-save-MOTION_THRESHOLD")
fill("#setting-MOTION_THRESHOLD", "0.37")
m.click("#setting-save-MOTION_THRESHOLD")
r.check("valeur valide enregistree", m.wait_for("(document.querySelector('#setting-status-MOTION_THRESHOLD')?.textContent || '').includes('Enregistré')", 8))
m.goto(url + "admin/settings")
m.wait_for("document.querySelector('#setting-MOTION_THRESHOLD') !== null", 10)
r.check("valeur conservee apres rechargement", m.js("return document.querySelector('#setting-MOTION_THRESHOLD').value") == "0.37")

# An integer refuses a decimal value (the browser lets step=1 through via JS, the server decides).
enabled("#setting-save-PROMETHEUS_ALERT_COOLDOWN")
fill("#setting-PROMETHEUS_ALERT_COOLDOWN", "1.5")
m.js("document.querySelector('#setting-PROMETHEUS_ALERT_COOLDOWN').removeAttribute('required'); document.querySelector('#setting-PROMETHEUS_ALERT_COOLDOWN').step = 'any'")
m.click("#setting-save-PROMETHEUS_ALERT_COOLDOWN")
r.check("entier invalide : message d'erreur", m.wait_for("(document.querySelector('#setting-status-PROMETHEUS_ALERT_COOLDOWN')?.textContent || '').includes('Entier')", 8),
        m.text("#setting-status-PROMETHEUS_ALERT_COOLDOWN"))

# Restores the original value.
enabled("#setting-save-MOTION_THRESHOLD")
fill("#setting-MOTION_THRESHOLD", original)
m.click("#setting-save-MOTION_THRESHOLD")
r.check("valeur d'origine retablie", m.wait_for("(document.querySelector('#setting-status-MOTION_THRESHOLD')?.textContent || '').includes('Enregistré')", 8))

# Secret: set then cleared; never displayed.
enabled("#setting-save-SMS_FREE_KEY")
fill("#setting-SMS_FREE_KEY", secret)
m.click("#setting-save-SMS_FREE_KEY")
r.check("secret enregistre : etat Defini", m.wait_for("(document.querySelector('#setting-state-SMS_FREE_KEY')?.textContent || '') === 'Défini'", 8), m.text("#setting-state-SMS_FREE_KEY"))
r.check("le champ secret est vide apres enregistrement", m.js("return document.querySelector('#setting-SMS_FREE_KEY').value") == "")
r.check("le secret n'apparait nulle part dans la page", secret not in m.js("return document.documentElement.outerHTML"))
m.goto(url + "admin/settings")
m.wait_for("document.querySelector('#setting-state-SMS_FREE_KEY') !== null", 10)
r.check("apres rechargement : Defini, secret absent du HTML",
        m.text("#setting-state-SMS_FREE_KEY") == "Défini" and secret not in m.js("return document.documentElement.outerHTML"))
enabled("#setting-clear-SMS_FREE_KEY")
m.click("#setting-clear-SMS_FREE_KEY")
r.check("secret efface : Non defini", m.wait_for("(document.querySelector('#setting-state-SMS_FREE_KEY')?.textContent || '') === 'Non défini'", 8), m.text("#setting-state-SMS_FREE_KEY"))

# ============================================== Application logs ===============================================
m.goto(url + "admin/logs")
r.check("page Journaux rendue avec des lignes", m.wait_for("document.querySelector('#logs-table') !== null", 15))
r.check("resume affiche (lignes, correspondantes, en memoire)",
        "en mémoire" in m.text("#logs-summary") or "in memory" in m.text("#logs-summary"), m.text("#logs-summary"))
r.check("le journal du demarrage est present",
        m.js("return [...document.querySelectorAll('#logs-table .vc-log-msg')].some(td => td.textContent.includes('HTTP server started'))") is True)
r.check("les lignes sont les plus recentes d'abord",
        m.js("const t = [...document.querySelectorAll('#logs-table .vc-log-time')].map(td => td.textContent); return t.length < 2 || t[0] >= t[t.length - 1];") is True)
FILTERED = ("document.querySelectorAll('#logs-table tbody tr').length >= 1 && [...document.querySelectorAll('#logs-table .vc-log-msg')]"
            ".every(td => td.textContent.toLowerCase().includes('http server started'))")
ok = False
for _ in range(20):  # until the interface is hydrated, the input is not taken into account
    fill("#logs-text", "HTTP server started")
    if m.wait_for(FILTERED, 1.5):
        ok = True
        break
r.check("filtre texte : seules les lignes correspondantes", ok)
fill("#logs-text", "xx-aucune-ligne-ne-contient-ceci-xx")
r.check("filtre sans resultat : message vide", m.wait_for("document.querySelector('#logs-empty') !== null", 8))
fill("#logs-text", "")
m.js("const s = document.querySelector('#logs-level'); s.value = 'error'; s.dispatchEvent(new Event('change', {bubbles: true}));")
r.check("niveau « erreurs seulement » : aucune ligne d'information",
        m.wait_for("document.querySelector('#logs-level').value === 'error' && document.querySelectorAll('#logs-table .vc-log--info').length === 0", 8))

r.finish()
