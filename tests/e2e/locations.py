#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page /admin/locations (composant CRUD generique) : ajout, validation serveur, modification, filtre,
suppression avec confirmation, traduction a chaud. Les emplacements crees sont supprimes a la fin.

Usage : python3 tests/e2e/locations.py url utilisateur_admin mot_de_passe
"""
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: locations.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
tag = f"E2E-{int(time.time())}"

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def rows():
    return m.js("return [...document.querySelectorAll('#crud-table tbody tr')].map(tr => tr.querySelector('td').textContent)") or []


def wait_rows(predicate_js, timeout=8):
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


# Clean state, then login through the API.
m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
m.goto(url + "admin/locations")
r.check("page rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("lien Emplacements actif dans la barre",
        m.js("return document.querySelector('#vc-nav-admin a')?.getAttribute('aria-current')") == "page")
r.check("formulaire ferme au depart", m.js("return document.querySelector('#crud-form')") is None)

# Adding ---------------------------------------------------------------------------------------------------------
name = f"{tag}-Salon"
for _ in range(40):  # until the interface is hydrated, the click opens nothing
    m.click("#crud-add")
    if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
        break
r.check("le formulaire s'ouvre", m.js("return document.querySelector('#crud-form') !== null"))
r.check("actif coche par defaut", m.js("return document.querySelector('#crud-field-active').checked") is True)
save_enabled()
fill("#crud-field-name", "   ")  # passes the browser's validation, not the server's
m.click("#crud-save")
r.check("nom vide cote serveur : erreur sous le champ",
        m.wait_for("(document.querySelector('#crud-error-name')?.textContent || '').includes('obligatoire')", 8),
        m.text("#crud-error-name"))
r.check("le formulaire reste ouvert", m.js("return document.querySelector('#crud-form') !== null"))
fill("#crud-field-name", name)
fill("#crud-field-description", "Rez-de-chaussee")
m.click("#crud-save")
r.check("ajout : ligne dans le tableau", wait_rows(f"rows.includes({name!r})"), str(rows()))
r.check("formulaire referme apres l'ajout", m.wait_for("document.querySelector('#crud-form') === null", 5))

# Editing ----------------------------------------------------------------------------------------------------------
renamed = f"{tag}-Salon-TV"
r.check("clic sur Modifier", click_in_row(name, ".crud-edit"))
r.check("formulaire prerempli", m.wait_for(f"document.querySelector('#crud-field-name')?.value === {name!r}", 5))
save_enabled()
fill("#crud-field-name", renamed)
m.js("document.querySelector('#crud-field-active').click()")  # unticked
m.click("#crud-save")
r.check("modification : nouveau nom", wait_rows(f"rows.includes({renamed!r}) && !rows.includes({name!r})"), str(rows()))
badge = m.js(
    "const tr = [...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === "
    f"{renamed!r}); return tr.querySelector('.vc-badge').textContent;"
)
r.check("badge Non apres decochage", badge == "Non", str(badge))

# Filter -----------------------------------------------------------------------------------------------------------
fill("#crud-search", "zzz-introuvable")
r.check("filtre sans resultat", m.wait_for("document.querySelector('#crud-no-match') !== null", 5))
fill("#crud-search", renamed.lower())
r.check("filtre insensible a la casse", wait_rows(f"rows.includes({renamed!r})") and m.js("return document.querySelector('#crud-no-match')") is None)
fill("#crud-search", "")

# Live translation -------------------------------------------------------------------------------------------------
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("titre et colonnes en anglais", m.wait_for(
    "document.querySelector('.vc-page-header h2').textContent === 'Locations' && "
    "[...document.querySelectorAll('#crud-table th')].some(t => t.textContent === 'Name')", 8))
badge = m.js(
    "const tr = [...document.querySelectorAll('#crud-table tbody tr')].find(t => t.querySelector('td').textContent === "
    f"{renamed!r}); return tr.querySelector('.vc-badge').textContent;"
)
r.check("badge traduit", badge == "No", str(badge))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")
m.wait_for("document.querySelector('.vc-page-header h2').textContent === 'Emplacements'", 8)

# Deletion with confirmation -------------------------------------------------------------------------------------
click_in_row(renamed, ".crud-delete")
r.check("confirmation demandee", m.wait_for("document.querySelector('.crud-confirm') !== null", 5))
r.check("l'element est toujours la", renamed in rows())
m.js("document.querySelector('.crud-confirm-no').click()")
r.check("annuler garde l'element", m.wait_for("document.querySelector('.crud-confirm') === null", 5) and renamed in rows())
click_in_row(renamed, ".crud-delete")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression confirmee", wait_rows(f"!rows.includes({renamed!r})"), str(rows()))

# Reload: the state does come from the server ---------------------------------------------------------------------
m.goto(url + "admin/locations")
r.check("apres rechargement, l'element reste supprime", m.wait_for("document.querySelector('#crud-add') !== null", 10) and renamed not in rows())

r.finish()
