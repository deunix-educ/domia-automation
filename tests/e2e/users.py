#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de la page /admin/users : creation, validation serveur, modification (identifiant en lecture seule),
reinitialisation du mot de passe, protections sur son propre compte, suppression. Le compte cree est supprime a la fin.

Usage : python3 tests/e2e/users.py url utilisateur_admin mot_de_passe
"""
import sys
import time

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: users.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
name = f"e2e{int(time.time())}"
pw1, pw2 = f"{name}-mot-de-passe-1", f"{name}-mot-de-passe-2"

m = Marionette()
r = Report()


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def select(selector, value):
    return m.js(f"const e = document.querySelector({selector!r}); e.value = {value!r}; e.dispatchEvent(new Event('change', {{bubbles: true}})); return e.value;")


def enabled(selector, timeout=15):
    return m.wait_for(f"document.querySelector({selector!r}) && !document.querySelector({selector!r}).disabled", timeout)


def row_js(user):
    return f"[...document.querySelectorAll('#crud-table tbody tr')].find(tr => tr.querySelector('td').textContent === {user!r})"


def has_row(user, timeout=8):
    return m.wait_for(f"!!{row_js(user)}", timeout)


def row_text(user):
    return m.js(f"const tr = {row_js(user)}; return tr ? tr.innerText.replace(/\\s+/g, ' ') : null")


def click_row(user, selector):
    return m.js(f"const tr = {row_js(user)}; if (!tr) return false; tr.querySelector({selector!r}).click(); return true;")


def banner():
    return m.js("return document.querySelector('#crud-banner')?.textContent || ''") or ""


def open_form(button="#crud-add"):
    for _ in range(40):
        m.click(button)
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


def login_probe(user, pw):
    """Tente une connexion sans toucher aux cookies du navigateur (credentials: omit). Renvoie le JSON de la fonction serveur."""
    body = f"username={user}&password={pw}"
    return m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch('/sf/login', {{method:'POST', credentials:'omit', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(x => x.text()).then(cb);"
    )


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
m.goto(url + "admin/users")
r.check("page Utilisateurs rendue", m.wait_for("document.querySelector('#crud-add') !== null", 10))
r.check("le compte administrateur est listé avec son rôle", has_row(admin) and "Administrateur" in (row_text(admin) or ""), str(row_text(admin)))
r.check("lien Utilisateurs dans la barre",
        m.js("return [...document.querySelectorAll('#vc-nav-admin a')].some(a => a.getAttribute('href') === '/admin/users' && a.getAttribute('aria-current') === 'page')") is True)

# Creation --------------------------------------------------------------------------------------------------------
r.check("formulaire ouvert", open_form())
r.check("rôle Utilisateur par défaut, actif coché",
        m.js("return document.querySelector('#crud-field-role').value") == "user" and m.js("return document.querySelector('#crud-field-is_active').checked") is True)
r.check("champ mot de passe masqué", m.js("return document.querySelector('#crud-field-password').type") == "password")
enabled("#crud-save")
fill("#crud-field-username", name)
fill("#crud-field-first_name", "Eve")
fill("#crud-field-last_name", "Test")
fill("#crud-field-email", "pas-un-email")
fill("#crud-field-mobile", "12ab")
fill("#crud-field-password", "court")
m.click("#crud-save")
r.check("e-mail invalide : erreur sous le champ", m.wait_for("document.querySelector('#crud-error-email') !== null", 8))
r.check("mobile invalide : erreur sous le champ", m.js("return document.querySelector('#crud-error-mobile') !== null") is True)
r.check("mot de passe trop court : motif traduit",
        "au moins 10" in (m.text("#crud-error-password") or ""), m.text("#crud-error-password"))
r.check("rien n'est créé", not m.js(f"return !!{row_js(name)}"))
fill("#crud-field-email", f"{name}@example.org")
fill("#crud-field-mobile", "+33 6 12 34 56 78")
fill("#crud-field-password", pw1)
enabled("#crud-save")
m.click("#crud-save")
r.check("ajout : ligne dans le tableau", has_row(name))
r.check("nom complet et rôle affichés", "Eve Test" in (row_text(name) or "") and "Utilisateur" in (row_text(name) or ""), str(row_text(name)))
r.check("le nouveau compte peut se connecter", '"Success"' in (login_probe(name, pw1) or "") or "Success" in (login_probe(name, pw1) or ""))

# Identifier already taken ------------------------------------------------------------------------------------------
r.check("formulaire rouvert", open_form())
enabled("#crud-save")
fill("#crud-field-username", name.upper())
fill("#crud-field-password", pw1)
m.click("#crud-save")
r.check("identifiant déjà pris (casse ignorée)", m.wait_for("(document.querySelector('#crud-error-username')?.textContent || '').includes('déjà')", 8), m.text("#crud-error-username"))
m.click("#crud-cancel")

# Editing: read-only identifier, role, password reset -----------------------------------------------------------------------
click_row(name, ".crud-edit")
r.check("formulaire prérempli", m.wait_for(f"document.querySelector('#crud-field-username')?.value === {name!r}", 5))
r.check("identifiant en lecture seule", m.js("return document.querySelector('#crud-field-username').readOnly") is True)
r.check("mot de passe jamais prérempli", m.js("return document.querySelector('#crud-field-password').value") == "")
enabled("#crud-save")
select("#crud-field-role", "admin")
fill("#crud-field-password", pw2)
m.click("#crud-save")
r.check("modification : rôle Administrateur", m.wait_for(f"(() => {{ const tr = {row_js(name)}; return !!tr && tr.innerText.includes('Administrateur'); }})()", 8), str(row_text(name)))
r.check("ancien mot de passe refusé", "InvalidCredentials" in (login_probe(name, pw1) or ""))
r.check("nouveau mot de passe accepté", "Success" in (login_probe(name, pw2) or ""))

# Protections on one's own account ------------------------------------------------------------------------------------
click_row(admin, ".crud-edit")
m.wait_for(f"document.querySelector('#crud-field-username')?.value === {admin!r}", 5)
enabled("#crud-save")
m.js("document.querySelector('#crud-field-is_active').click()")  # unticked
m.click("#crud-save")
r.check("se désactiver soi-même : refusé", m.wait_for("(document.querySelector('#crud-banner')?.textContent || '').includes('propre compte')", 8), banner())
r.check("le formulaire reste ouvert, aucune session coupée", m.js("return document.querySelector('#crud-form') !== null") is True)
m.click("#crud-cancel")
click_row(admin, ".crud-delete")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("se supprimer soi-même : refusé", m.wait_for("(document.querySelector('#crud-banner')?.textContent || '').includes('propre compte')", 8), banner())
r.check("le compte administrateur est toujours là", has_row(admin))

# Live translation --------------------------------------------------------------------------------------------------
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("rôle traduit dans le tableau", m.wait_for(f"(() => {{ const tr = {row_js(name)}; return !!tr && tr.innerText.includes('Administrator'); }})()", 8))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")
m.wait_for("document.querySelector('.vc-page-header h2').textContent === 'Utilisateurs'", 8)

# Deletion ------------------------------------------------------------------------------------------------------------
click_row(name, ".crud-delete")
m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 5)
m.js("document.querySelector('.crud-confirm-yes').click()")
r.check("suppression : plus dans le tableau", m.wait_for(f"!{row_js(name)}", 8))
r.check("le compte supprimé ne peut plus se connecter", "InvalidCredentials" in (login_probe(name, pw2) or ""))

r.finish()
