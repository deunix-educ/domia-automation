#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E du mouvement et de la detection d'objets (R6 lot 8) : un VRAI flux H.264 (image du bus, luminosite oscillante, ffmpeg -> MediaMTX de dev) regarde
par une camera en mode « mouvement » avec YOLO. Verifie : un mouvement est enregistre, une detection « bus » (classe voulue) apparait avec son image
annotee (JPEG servi par l'application), la page des evenements, et le reglage global du detecteur (coupe : plus de detection).

Prerequis : ffmpeg avec libx264, MediaMTX de la stack de dev, modele (make fetch-model : sinon la detection n'est pas verifiee, le mouvement l'est).
Usage : python3 tests/e2e/events.py url utilisateur_admin mot_de_passe      (variables : E2E_YOLO_MODEL, E2E_BUS_IMAGE)
"""
import http.cookiejar
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: events.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
base = url.rstrip("/")
rtsp_port = os.environ.get("MEDIAMTX_RTSP_PORT", "18554")
internal_rtsp_port = os.environ.get("MEDIAMTX_INTERNAL_RTSP_PORT", "8554")
bus = os.environ.get("E2E_BUS_IMAGE", "")
have_model = os.path.exists(os.environ.get("E2E_YOLO_MODEL", "/nonexistent"))

m = Marionette()
r = Report()
if not shutil.which("ffmpeg") or not os.path.exists(bus):
    print("NOTE : ffmpeg ou l'image de reference absents, mouvement et detection NON verifies ici.")
    r.finish()
    sys.exit(0)
if not have_model:
    print("NOTE : modele YOLO absent (make fetch-model) : la detection n'est PAS verifiee ici, seulement le mouvement.")

jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(method, path, data=None, headers=None):
    h = {"Origin": base}
    h.update(headers or {})
    req = urllib.request.Request(base + path, data=data, method=method, headers=h)
    try:
        with opener.open(req, timeout=30) as resp:
            return resp.status, resp.headers, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def post_json(path, body):
    status, _, raw = call("POST", path, json.dumps(body).encode(), {"Content-Type": "application/json"})
    return status, raw.decode()


def form(path, fields):
    data = urllib.parse.urlencode({k: str(v).lower() if isinstance(v, bool) else v for k, v in fields.items()}).encode()
    status, _, raw = call("POST", path, data, {"Content-Type": "application/x-www-form-urlencoded"})
    return status, raw.decode()


status, _ = post_json("/api/auth/login", {"username": user, "password": password})
r.check("connexion administrateur", status == 200, str(status))

status, _ = form("/sf/detector/set", {"enabled": have_model, "save_frames": True})
r.check("detecteur regle (actif, images gardees)", status == 200, str(status))
status, text = form("/sf/detector/get", {})
r.check("... et relu", status == 200 and json.loads(text) == {"enabled": have_model, "save_frames": True}, text)

source = f"e2e-evt-src-{os.getpid()}"
camera = {
    "id": None, "name": f"E2E mouvement {os.getpid()}", "location_id": None, "active": True, "priority": 1, "notes": "",
    "proto": "rtsp", "ip": "127.0.0.1", "port": int(internal_rtsp_port), "path": source, "rtsp_user": "", "rtsp_pass": "", "fps": 10,
    "rtsp_any_port": False, "rtsp_transport_tcp": False, "recording_mode": "motion", "night_mode_api": "none", "web_port": 80,
    "web_api_key": "", "web_user": "", "web_pass": "", "onvif": False, "onvif_port": 80, "onvif_user": "", "onvif_pass": "", "ptz": False,
    "yolo_enabled": True, "yolo_classes": "bus", "yolo_min_confidence_pct": 30,
}
status, text = post_json("/sf/cameras/save", {"input": camera})
r.check("camera creee en mode mouvement avec YOLO (classe : bus)", status == 200 and "Saved" in text, text[:120])
cam = json.loads(text)["Saved"] if status == 200 and "Saved" in text else {"id": 0, "uuid": "0" * 32}
uuid = cam["uuid"]


def events():
    status, text = form("/sf/events/list", {"camera": uuid})
    return json.loads(text) if status == 200 else []


def wait_event(kind, timeout):
    end = time.time() + timeout
    while time.time() < end:
        found = [e for e in events() if e["kind"] == kind]
        if found:
            return found
        time.sleep(2)
    return []


ff = None
try:
    # The bus image's brightness oscillates (0.6 Hz): it is a motion, and the bus stays recognisable.
    ff = subprocess.Popen(
        ["ffmpeg", "-loglevel", "error", "-re", "-loop", "1", "-framerate", "10", "-i", bus,
         "-vf", "scale=640:480,eq=brightness='0.25*sin(2*PI*t*0.6)':eval=frame,format=yuv420p",
         "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-g", "10", "-profile:v", "baseline",
         "-f", "rtsp", "-rtsp_transport", "tcp", f"rtsp://127.0.0.1:{rtsp_port}/{source}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    motion = wait_event("motion", 90)
    r.check("un mouvement est enregistre", bool(motion), str(events()[:2]))
    r.check("... avec une part d'image chiffree", bool(motion) and (motion[0].get("motion_pct") or 0) > 1.0, str(motion[:1]))

    if have_model:
        det = wait_event("detection", 90)
        r.check("une detection « bus » est enregistree", bool(det) and "bus" in det[0]["classes"], str(det[:1]))
        r.check("... avec son image annotee", bool(det) and det[0].get("frame_id") is not None, str(det[:1]))
        if det and det[0].get("frame_id") is not None:
            status, headers, body = call("GET", f"/surveillance/detections/{det[0]['frame_id']}/frame/")
            r.check("l'image annotee est un JPEG servi par l'application",
                    status == 200 and headers.get("Content-Type") == "image/jpeg" and body[:2] == b"\xff\xd8", f"{status} {len(body)}")
        status, _, _ = call("GET", "/surveillance/detections/999999999/frame/")
        r.check("detection inconnue : 404", status == 404, str(status))

    # ----------------------------------------------------------------------------------------------- page
    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )
    m.goto(f"{url}surveillance/events")
    r.check("page des evenements : tableau", m.wait_for("document.querySelectorAll('#events-table tbody tr').length >= 1", 20))
    r.check("... avec un mouvement", m.js("return document.querySelector('#events-table tbody tr[data-kind=motion]') !== null"))
    r.check("reglage du detecteur visible pour l'administrateur", m.wait_for("document.querySelector('#detector-card') !== null", 10))
    # Turns the detector off from the interface (the page reactivates itself and loads the setting: we wait for the expected state, we retry the clicks).
    if have_model:
        r.check("la case reflete le reglage actif", m.wait_for("document.querySelector('#detector-enabled')?.checked === true", 15))
    for _ in range(8):
        if m.js("return document.querySelector('#detector-enabled')?.checked === false"):
            break
        m.click("#detector-enabled")
        time.sleep(0.5)
    saved = False
    for _ in range(8):
        m.click("#detector-save")
        if m.wait_for("document.querySelector('#detector-saved') !== null", 2):
            saved = True
            break
    r.check("coupure du detecteur enregistree", saved)
    status, text = form("/sf/detector/get", {})
    r.check("... et relue", status == 200 and json.loads(text)["enabled"] is False, text)

    if have_model:
        before = len([e for e in events() if e["kind"] == "detection"])
        time.sleep(25)
        after = len([e for e in events() if e["kind"] == "detection"])
        r.check("detecteur coupe : plus de nouvelle detection", after == before, f"{before} -> {after}")
finally:
    if ff and ff.poll() is None:
        ff.terminate()
        try:
            ff.wait(5)
        except subprocess.TimeoutExpired:
            ff.kill()
    if cam["id"]:
        form("/sf/cameras/delete", {"id": cam["id"]})

r.finish()
