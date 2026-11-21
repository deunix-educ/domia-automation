#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des enregistrements (R6 lot 7) : un VRAI flux H.264 (ffmpeg -> MediaMTX de dev) enregistre par MediaMTX en segments fMP4 de 10 s, indexes puis
archives dans ReductStore par le role video. Verifie : le path de la camera enregistre (mode continu), les segments apparaissent en base puis
« archives », le replay (200 + signature MP4, 206 pour une plage, memes octets), la page des enregistrements, l'enregistrement manuel (demarrage /
arret par l'interface, type `manual`) et la suppression.

Prerequis : ffmpeg avec libx264, MediaMTX et ReductStore de la stack de dev (make dev-up), dossier E2E_RECORDINGS_DIR commun au serveur et a MediaMTX.
Usage : python3 tests/e2e/recordings.py url utilisateur_admin mot_de_passe
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
    sys.exit("usage: recordings.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
base = url.rstrip("/")
api = os.environ.get("MEDIAMTX_API_URL", "http://127.0.0.1:19997")
rtsp_port = os.environ.get("MEDIAMTX_RTSP_PORT", "18554")
internal_rtsp_port = os.environ.get("MEDIAMTX_INTERNAL_RTSP_PORT", "8554")

m = Marionette()
r = Report()
if not shutil.which("ffmpeg"):
    print("NOTE : ffmpeg absent, les enregistrements NE sont PAS verifies ici.")
    r.finish()
    sys.exit(0)

jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(method, path, body=None, headers=None):
    data = json.dumps(body).encode() if isinstance(body, (dict, list)) else body
    h = {"Content-Type": "application/json", "Origin": base}
    h.update(headers or {})
    req = urllib.request.Request(base + path, data=data, method=method, headers=h)
    try:
        with opener.open(req, timeout=30) as resp:
            return resp.status, resp.headers, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def post(path, body):
    status, _, raw = call("POST", path, body)
    return status, raw.decode()


def form(path, fields):
    """Fonctions serveur a arguments simples : corps `application/x-www-form-urlencoded`."""
    data = urllib.parse.urlencode({k: str(v).lower() if isinstance(v, bool) else v for k, v in fields.items()}).encode()
    status, _, raw = call("POST", path, data, {"Content-Type": "application/x-www-form-urlencoded"})
    return status, raw.decode()


status, _ = post("/api/auth/login", {"username": user, "password": password})
r.check("connexion administrateur", status == 200, str(status))

source = f"e2e-rec-src-{os.getpid()}"
camera = {
    "id": None, "name": f"E2E enregistrement {os.getpid()}", "location_id": None, "active": True, "priority": 1, "notes": "",
    "proto": "rtsp", "ip": "127.0.0.1", "port": int(internal_rtsp_port), "path": source, "rtsp_user": "", "rtsp_pass": "", "fps": 10,
    "rtsp_any_port": False, "rtsp_transport_tcp": False, "recording_mode": "continuous", "night_mode_api": "none", "web_port": 80,
    "web_api_key": "", "web_user": "", "web_pass": "", "onvif": False, "onvif_port": 80, "onvif_user": "", "onvif_pass": "", "ptz": False,
    "yolo_enabled": False, "yolo_classes": "", "yolo_min_confidence_pct": 50,
}
status, text = post("/sf/cameras/save", {"input": camera})
r.check("camera creee en enregistrement continu", status == 200 and "Saved" in text, text[:120])
cam = json.loads(text)["Saved"] if status == 200 and "Saved" in text else {"id": 0, "uuid": "0" * 32}
uuid = cam["uuid"]

ff = None


def path_conf():
    try:
        with urllib.request.urlopen(f"{api}/v3/config/paths/get/{uuid}", timeout=5) as resp:
            return json.load(resp)
    except Exception:
        return {}


def day_list():
    status, text = form("/sf/recordings/list", {"uuid": uuid, "day": ""})
    if status != 200:
        return None
    return json.loads(text)


def items(kind=None, status=None):
    data = day_list()
    out = (data or {}).get("items", [])
    return [i for i in out if (kind is None or i["event_type"] == kind) and (status is None or i["status"] == status)]


def wait_items(count, kind=None, status="done", timeout=100):
    end = time.time() + timeout
    while time.time() < end:
        if len(items(kind, status)) >= count:
            return True
        time.sleep(2)
    return False


try:
    ff = subprocess.Popen(
        ["ffmpeg", "-loglevel", "error", "-re", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=25",
         "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-g", "25", "-pix_fmt", "yuv420p",
         "-profile:v", "baseline", "-f", "rtsp", "-rtsp_transport", "tcp", f"rtsp://127.0.0.1:{rtsp_port}/{source}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    r.check("le path de la camera enregistre (record: true)", path_conf().get("record") is True, str(path_conf().get("record")))

    # ------------------------------------------------------------------------------------------------ continu : segments indexes puis archives
    r.check("deux segments continus archives dans ReductStore", wait_items(2, "continuous"), str(items()))
    done = items("continuous", "done")
    rec = done[0] if done else {"id": 0, "size_bytes": 0}
    r.check("le segment a une taille et une duree", (rec.get("size_bytes") or 0) > 1000 and (rec.get("duration_ms") or 0) >= 5000, str(rec))

    # ------------------------------------------------------------------------------------------------ replay
    stream = f"/surveillance/recordings/{uuid}/{rec['id']}/stream/"
    status, headers, body = call("GET", stream)
    r.check("replay : 200 video/mp4", status == 200 and headers.get("Content-Type") == "video/mp4", f"{status} {headers.get('Content-Type')}")
    r.check("replay : signature MP4 (ftyp)", body[4:8] == b"ftyp", repr(body[:12]))
    r.check("replay : la taille correspond a l'index", len(body) == rec.get("size_bytes"), f"{len(body)} / {rec.get('size_bytes')}")
    status, headers, part = call("GET", stream, headers={"Range": "bytes=0-99"})
    r.check("replay progressif : 206 et 100 octets", status == 206 and len(part) == 100 and part == body[:100], f"{status} {len(part)}")
    r.check("... avec Content-Range", (headers.get("Content-Range") or "").startswith("bytes 0-99/"), headers.get("Content-Range") or "")
    status, _, _ = call("GET", f"/surveillance/recordings/{uuid}/999999999/stream/")
    r.check("segment inconnu : 404", status == 404, str(status))

    # ------------------------------------------------------------------------------------------------ page
    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )
    m.goto(f"{url}surveillance/recordings/{uuid}")
    r.check("page des enregistrements : tableau des segments", m.wait_for("document.querySelectorAll('#recordings-table tbody tr').length >= 2", 20))
    r.check("les segments sont marques « archives »", m.js("return document.querySelector('#recordings-table tbody tr').dataset.status") == "done")
    for _ in range(5):
        m.click("#recordings-table .rec-play")
        if m.wait_for("document.querySelector('#rec-player') !== null", 3):
            break
    r.check("« Lire » ouvre le lecteur sur le flux du segment",
            m.wait_for("(document.querySelector('#rec-player')?.getAttribute('src') || '').includes('/stream/')", 10))
    # Regression : la liste se rafraichit toute seule (5 s) ; le lecteur ne doit PAS etre recree (la video repartait du debut).
    m.js("document.querySelector('#rec-player').dataset.mark = 'same'")
    time.sleep(7)
    r.check("le lecteur survit au rafraichissement de la liste", m.js("return document.querySelector('#rec-player')?.dataset.mark") == "same")

    # ------------------------------------------------------------------------------------------------ manuel
    camera["id"] = cam["id"]
    camera["recording_mode"] = "manual"
    status, text = post("/sf/cameras/save", {"input": camera})
    r.check("camera passee en mode manuel", status == 200 and "Saved" in text, text[:100])
    r.check("en mode manuel, le path n'enregistre plus", path_conf().get("record") is not True, str(path_conf().get("record")))
    # L'enregistrement continu est fini : on attend que ses derniers segments soient pris, puis on compte.
    time.sleep(3)
    before = len(items("manual"))
    status, text = form("/sf/recordings/manual", {"uuid": uuid, "on": True})
    r.check("demarrage manuel accepte", status == 200 and "Done" in text, text[:100])
    r.check("MediaMTX enregistre (record: true)", path_conf().get("record") is True)
    m.goto(f"{url}surveillance/recordings/{uuid}")
    r.check("la page affiche « enregistrement en cours »", m.wait_for("document.querySelector('#rec-manual-state') !== null", 20))
    time.sleep(14)
    status, text = form("/sf/recordings/manual", {"uuid": uuid, "on": False})
    r.check("arret manuel accepte", status == 200 and "Done" in text, text[:100])
    r.check("MediaMTX n'enregistre plus", path_conf().get("record") is not True)
    r.check("un segment de type manuel est archive", wait_items(before + 1, "manual", timeout=90), str(items()))

    status, text = form("/sf/recordings/manual", {"uuid": "0" * 32, "on": True})
    r.check("camera inconnue : NotFound", "NotFound" in text, text[:80])

    # ------------------------------------------------------------------------------------------------ suppression
    victim = (items("continuous", "done") or [rec])[0]
    count = len(items())
    status, text = form("/sf/recordings/delete", {"uuid": uuid, "id": victim["id"]})
    r.check("suppression d'un enregistrement", status == 200 and "Deleted" in text, text[:100])
    r.check("... il disparait de la liste", len(items()) == count - 1)
    status, _, _ = call("GET", f"/surveillance/recordings/{uuid}/{victim['id']}/stream/")
    r.check("... et son flux n'existe plus (404)", status == 404, str(status))
    status, text = form("/sf/recordings/delete", {"uuid": uuid, "id": victim["id"]})
    r.check("supprimer deux fois : NotFound", "NotFound" in text, text[:80])
finally:
    if ff and ff.poll() is None:
        ff.terminate()
        try:
            ff.wait(5)
        except subprocess.TimeoutExpired:
            ff.kill()
    if cam["id"]:
        req = urllib.request.Request(base + "/sf/cameras/delete", data=f"id={cam['id']}".encode(), method="POST",
                                     headers={"Content-Type": "application/x-www-form-urlencoded", "Origin": base})
        try:
            opener.open(req, timeout=15).read()
        except Exception:
            pass

r.finish()
