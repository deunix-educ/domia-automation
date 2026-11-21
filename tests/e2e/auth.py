#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E de l'authentification dans le navigateur : garde, formulaire de connexion, page Mon compte, changement de mot
de passe (aller-retour : le mot de passe d'origine est retabli a la fin), deconnexion.

Usage : python3 tests/e2e/auth.py url utilisateur mot_de_passe
Prerequis : serveur demarre, compte existant, Firefox en mode Marionette.
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: auth.py url utilisateur mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
temp = password + "-tmp-9"

m = Marionette()
r = Report()


def path():
    return m.js("return location.pathname + location.search")


def clear():
    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    m.js("localStorage.clear()")


def fill(selector, value):
    # Saisie realiste : l'evenement `input` met a jour l'etat de l'interface.
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def sign_in(u, p, expect_ok=True):
    """Remplit et soumet le formulaire de connexion, une fois le bouton actif (= interface hydratee)."""
    if not m.wait_for("document.querySelector('#login-submit') && !document.querySelector('#login-submit').disabled", 15):
        return False
    fill("#login-username", u)
    fill("#login-password", p)
    m.click("#login-submit")
    if expect_ok:
        return m.wait_for("location.pathname !== '/login'", 8)
    return m.wait_for("document.querySelector('#login-error') !== null", 8)


# --- garde -----------------------------------------------------------------------------------------------------
clear()
m.goto(url + "account")
r.check("page protegee : renvoi vers /login avec la cible", path() == "/login?next=%2Faccount", path())
r.check("aucune barre laterale pour un anonyme", m.js("return document.querySelector('.vc-sidebar')") is None)

# --- remplissage automatique : valeurs posees SANS evenement `input` (gestionnaire de mots de passe, restauration de formulaire) ---
m.goto(url + "login")
m.wait_for("document.querySelector('#login-submit') && !document.querySelector('#login-submit').disabled", 20)
m.js(f"document.querySelector('#login-username').value = {user!r}; document.querySelector('#login-password').value = {password!r};")
m.click("#login-submit")
r.check("champs remplis sans evenement input : connexion reussie", m.wait_for("location.pathname !== '/login'", 8), path())
m.js_async("const cb = arguments[arguments.length-1]; fetch('/api/auth/logout', {method:'POST'}).then(() => cb(1));")
m.goto(url + "account")
r.check("(retour a l'etat anonyme)", path() == "/login?next=%2Faccount", path())

# --- connexion : echec puis succes ----------------------------------------------------------------------------
r.check("mauvais mot de passe : message d'erreur", sign_in(user, "mauvais mot de passe", expect_ok=False))
msg = m.text("#login-error") or ""
r.check("message en francais", "incorrect" in msg, msg)
r.check("mot de passe efface apres l'echec", m.js("return document.querySelector('#login-password').value") == "")
r.check("toujours sur /login", m.js("return location.pathname") == "/login")
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /English/i.test(b.textContent)).click()")
r.check("message traduit a chaud", m.wait_for("document.querySelector('#login-error').textContent.includes('Incorrect')", 5),
        m.text("#login-error"))
m.js("[...document.querySelectorAll('.vc-lang button')].find(b => /Fran/i.test(b.textContent)).click()")

r.check("bon mot de passe : connexion", sign_in(user, password))
r.check("retour vers la page demandee (/account)", m.wait_for("location.pathname === '/account'", 8), path())
r.check("barre laterale apparue sans rechargement", m.wait_for("document.querySelector('.vc-sidebar') !== null", 8))
r.check("identite affichee", user in (m.text("#account-identity") or ""), m.text("#account-identity"))
r.check("lien Mon compte actif dans la barre",
        m.js("return document.querySelector('a.vc-sidebar-user')?.getAttribute('aria-current')") == "page")

# --- /login quand on est deja connecte ---------------------------------------------------------------------
m.goto(url + "login")
r.check("deja connecte : /login renvoie vers l'accueil", path() == "/", path())
m.goto(url + "login?next=%2F%2Fevil.example")
r.check("redirection ouverte refusee", m.js("return location.hostname") == "127.0.0.1" and path() == "/", path())

# --- changement de mot de passe ------------------------------------------------------------------------------
m.goto(url + "account")


def change(current, new, confirm):
    m.wait_for("document.querySelector('#pw-submit') && !document.querySelector('#pw-submit').disabled", 10)
    fill("#pw-current", current)
    fill("#pw-new", new)
    fill("#pw-confirm", confirm)
    m.click("#pw-submit")


m.wait_for("document.querySelector('#pw-submit') && !document.querySelector('#pw-submit').disabled", 15)
change("pas le bon", temp, temp)
r.check("ancien mot de passe faux : refuse", m.wait_for("(document.querySelector('#password-error')?.textContent || '').includes('actuel')", 8),
        m.text("#password-error"))
change(password, temp, temp + "x")
r.check("confirmation differente : refuse sans appel serveur",
        m.wait_for("(document.querySelector('#password-error')?.textContent || '').includes('différents')", 8), m.text("#password-error"))
change(password, "court", "court")
r.check("mot de passe trop court : motif traduit",
        m.wait_for("(document.querySelector('#password-error')?.textContent || '').includes('au moins')", 8), m.text("#password-error"))
change(password, temp, temp)
r.check("changement accepte : retour sur /login avec avis",
        m.wait_for("location.pathname === '/login' && document.querySelector('#login-notice') !== null", 10), path())
r.check("session fermee : plus de barre laterale", m.wait_for("document.querySelector('.vc-sidebar') === null", 8))
m.goto(url + "account")
r.check("l'ancienne session ne donne plus acces", path() == "/login?next=%2Faccount", path())

# Aller-retour : on se reconnecte avec le mot de passe temporaire puis on retablit l'original.
r.check("connexion avec le nouveau mot de passe", sign_in(user, temp))
m.goto(url + "account")
m.wait_for("document.querySelector('#pw-submit') && !document.querySelector('#pw-submit').disabled", 15)
change(temp, password, password)
r.check("mot de passe d'origine retabli",
        m.wait_for("location.pathname === '/login' && document.querySelector('#login-notice') !== null", 10), path())
r.check("reconnexion avec le mot de passe d'origine", sign_in(user, password))

# --- deconnexion -----------------------------------------------------------------------------------------------
done = False
for _ in range(30):
    m.click("#vc-logout")
    if m.wait_for("location.pathname === '/login' && document.querySelector('.vc-sidebar') === null", 1.5):
        done = True
        break
r.check("deconnexion : retour sur /login, barre laterale disparue", done, path())
m.goto(url)
r.check("apres deconnexion, la racine renvoie vers /login", path() == "/login", path())

# --- cible hors de l'interface (documentation) : vrai chargement de page, pas la page « introuvable » du routeur ---
m.goto(url + "docs/")
r.check("documentation sans session : renvoi vers /login avec la cible", path() == "/login?next=%2Fdocs%2F", path())
sign_in(user, password)
r.check("apres connexion : /docs/ charge par le serveur (pas la page introuvable de l'interface)",
        m.wait_for("location.pathname === '/docs/' && document.querySelector('.vc-main-wrap, .vc-auth-wrap') === null", 10), path())
m.goto(url)
# Meme cible depuis la barre de navigation : lien ordinaire (rel="external"), pas une navigation du routeur.
if m.wait_for("document.querySelector('a.vc-nav-link[href=\"/docs/\"]') !== null", 10):
    done = False
    for _ in range(20):
        m.click('a.vc-nav-link[href="/docs/"]')
        if m.wait_for("location.pathname === '/docs/' && document.querySelector('.vc-main-wrap, .vc-auth-wrap') === null", 1.5):
            done = True
            break
    r.check("lien Documentation de la barre : /docs/ charge par le serveur", done, path())
    m.goto(url)
r.check("aucune entree de navigation vers une page inexistante (/observability/)",
        m.wait_for("document.querySelector('.vc-sidebar') !== null", 10)
        and not m.js("return document.querySelector('a.vc-nav-link[href=\"/observability/\"]') !== null"), path())

r.finish()
