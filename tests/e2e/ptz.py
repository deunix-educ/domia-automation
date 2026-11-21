#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des commandes d'une camera sur la page live (R6 lot 5) : pilotage PTZ par le WebSocket (direction maintenue = ordres repetes,
relachement = arret, zoom, vitesse), presets (enregistrer, aller, supprimer) et bascule jour / nuit, contre une camera ONVIF simulee
(tools/sim-onvif.py) dont chaque ordre est journalise.

Usage : python3 tests/e2e/ptz.py url utilisateur_admin mot_de_passe
Variables : E2E_ONVIF_PORT, E2E_ONVIF_PASSWORD, E2E_ONVIF_LOG (poses par tools/e2e.sh).
"""
import http.cookiejar
import json
import os
import sys
import time
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: ptz.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
base = url.rstrip("/")
port, secret, log_path = os.environ.get("E2E_ONVIF_PORT"), os.environ.get("E2E_ONVIF_PASSWORD"), os.environ.get("E2E_ONVIF_LOG")
if not (port and secret and log_path):
    sys.exit("E2E_ONVIF_* absents : lancer par tools/e2e.sh")

m = Marionette()
r = Report()
jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(path, body):
    req = urllib.request.Request(base + path, data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json", "Origin": base})
    with opener.open(req, timeout=20) as resp:
        return resp.status, resp.read().decode()


def commands():
    try:
        return open(log_path).read().splitlines()
    except FileNotFoundError:
        return []


def wait_command(predicate, timeout=10):
    end = time.time() + timeout
    while time.time() < end:
        if any(predicate(c) for c in commands()):
            return True
        time.sleep(0.1)
    return False


def pointer(selector, kind):
    m.js(f"document.querySelector({selector!r}).dispatchEvent(new PointerEvent({kind!r}, {{bubbles: true, pointerId: 1}}))")


def fill(selector, value):
    m.js(f"const e = document.querySelector({selector!r}); e.focus(); e.value = {value!r}; e.dispatchEvent(new Event('input', {{bubbles: true}}))")


cam_id = 0
try:
    status, _ = call("/api/auth/login", {"username": user, "password": password})
    r.check("connexion administrateur", status == 200, str(status))
    # La camera simulee est ajoutee par son adresse (identifiants fournis : elle est synchronisee, donc PTZ detecte).
    status, text = call("/sf/cameras/add-by-ip", {"input": {"ip": "127.0.0.1", "onvif_port": int(port), "user": "admin", "pass": secret}})
    added = json.loads(text)["Added"]
    cam_id = added["id"]
    r.check("camera ajoutee et reconnue PTZ", status == 200 and added["model"] == "IPC_NT98566", text[:120])
    # Methode jour / nuit : ONVIF (la camera simulee sait l'imagerie).
    status, text = call("/sf/cameras/list", {})
    cam = next(c for c in json.loads(text) if c["id"] == cam_id)
    cam.pop("has_rtsp_pass", None)
    body = {k: cam[k] for k in (
        "id", "name", "location_id", "active", "priority", "notes", "proto", "ip", "port", "path", "rtsp_user", "fps",
        "rtsp_any_port", "rtsp_transport_tcp", "recording_mode", "night_mode_api", "web_port", "web_user", "onvif", "onvif_port",
        "onvif_user", "yolo_enabled", "yolo_classes", "yolo_min_confidence_pct")}
    body.update({"night_mode_api": "onvif", "rtsp_pass": "", "web_api_key": "", "web_pass": "", "onvif_pass": "", "ptz": cam["ptz"]})
    status, text = call("/sf/cameras/save", {"input": body})
    r.check("jour / nuit configure en ONVIF", status == 200 and "Saved" in text, text[:100])
    uuid = cam["uuid"]

    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )
    m.goto(f"{url}surveillance/live/{uuid}")
    r.check("page live avec ses commandes", m.wait_for("document.querySelector('#ptz-panel') !== null && document.querySelector('#night-panel') !== null", 20))
    r.check("le pilotage se connecte a la camera", m.wait_for("document.querySelector('#ptz-panel')?.dataset.ptzState === 'ready'", 60),
            m.text("#ptz-status") or "")

    # Direction maintenue : l'ordre est renvoye ; relachement : arret.
    pointer("#ptz-right", "pointerdown")
    r.check("droite : ContinuousMove a mi-vitesse", wait_command(lambda c: c == "ContinuousMove 0.5 0 0"), str(commands()[-3:]))
    n = len([c for c in commands() if c.startswith("ContinuousMove")])
    time.sleep(1.0)
    r.check("l'ordre est renvoye tant que le bouton est maintenu", len([c for c in commands() if c.startswith("ContinuousMove")]) >= n + 2)
    pointer("#ptz-right", "pointerup")
    r.check("relachement : Stop", wait_command(lambda c: c == "Stop"))
    after = len(commands())
    time.sleep(0.8)
    r.check("plus aucun ordre apres le relachement", len(commands()) == after)

    # Vitesse et autres directions.
    fill("#ptz-speed-pt", "100")
    pointer("#ptz-up", "pointerdown")
    r.check("haut a pleine vitesse : tilt +1", wait_command(lambda c: c == "ContinuousMove 0 1 0"), str(commands()[-3:]))
    stops = commands().count("Stop")
    pointer("#ptz-up", "pointerleave")
    r.check("sortie du bouton : arret aussi", wait_command(lambda c: commands().count("Stop") > stops) and commands()[-1] == "Stop")
    pointer("#ptz-zoom-in", "pointerdown")
    r.check("zoom avant", wait_command(lambda c: c == "ContinuousMove 0 0 0.5"), str(commands()[-3:]))
    pointer("#ptz-zoom-in", "pointerup")
    m.click("#ptz-stop")

    # Presets.
    r.check("aucun preset au depart", m.js("return document.querySelector('#ptz-no-presets') !== null"))
    fill("#ptz-preset-name", "Porte d'entrée")
    m.click("#ptz-preset-save")
    r.check("preset enregistre dans la camera", wait_command(lambda c: c == "SetPreset Porte d'entrée"))
    r.check("le preset apparait dans la liste", m.wait_for("document.querySelector('#ptz-presets li') !== null", 10))
    r.check("le nom est affiche", "Porte d'entrée" in (m.text("#ptz-presets li span") or ""))
    m.click("#ptz-presets .ptz-goto")
    r.check("aller au preset", wait_command(lambda c: c.startswith("GotoPreset preset_")))
    m.click("#ptz-presets .ptz-remove")
    r.check("preset supprime de la camera", wait_command(lambda c: c.startswith("RemovePreset preset_")))
    r.check("... et de la liste", m.wait_for("document.querySelector('#ptz-presets li') === null", 10))

    # Jour / nuit.
    m.click("#night-ircut")
    r.check("IR-cut : la camera passe en NIGHT", wait_command(lambda c: "IrCutFilter=NIGHT" in c), str(commands()[-2:]))
    r.check("le retour est affiche", m.wait_for("(document.querySelector('#night-feedback')?.textContent || '').includes('NIGHT')", 10),
            m.text("#night-feedback") or "")
    m.click("#night-ircut")
    r.check("deuxieme appui : retour a DAY", wait_command(lambda c: "IrCutFilter=DAY" in c))
    m.click("#night-light")
    r.check("lumiere : luminosite a 80", wait_command(lambda c: "Brightness=80" in c), str(commands()[-2:]))
    html = m.js("return document.documentElement.outerHTML")
    r.check("aucun mot de passe dans la page", secret not in html)
finally:
    if cam_id:
        try:
            req = urllib.request.Request(base + "/sf/cameras/delete", data=f"id={cam_id}".encode(), method="POST",
                                         headers={"Content-Type": "application/x-www-form-urlencoded", "Origin": base})
            opener.open(req, timeout=15).read()
        except Exception:
            pass

r.finish()
