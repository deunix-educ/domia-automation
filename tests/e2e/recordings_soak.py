#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Endurance de l'enregistrement continu (R6 lot 7, critere d'acceptation : « 1 h d'enregistrement continu sans perte de segment »).

Un VRAI flux H.264 (ffmpeg -> MediaMTX de dev) est enregistre en continu pendant SOAK_SECONDS (defaut 3600) ; a la fin on verifie, sur l'index
de DomiNet et dans ReductStore : aucun trou entre segments consecutifs, aucun segment en erreur, tous archives, duree cumulee >= 97 % de la
duree publiee, et chaque segment relu depuis ReductStore est un MP4 de la taille indexee. Le rapport detaille est imprime (nombre de segments,
plus grand ecart, taille totale).

Non inclus dans la liste par defaut de tools/e2e.sh (une heure) : `SOAK_SECONDS=3600 tools/e2e.sh recordings_soak`.
Usage : python3 tests/e2e/recordings_soak.py url utilisateur_admin mot_de_passe
"""
import datetime
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
    sys.exit("usage: recordings_soak.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
base = url.rstrip("/")
duration = int(os.environ.get("SOAK_SECONDS", "3600"))
segment_s = int(os.environ.get("SOAK_SEGMENT_SECONDS", "10"))
rtsp_port = os.environ.get("MEDIAMTX_RTSP_PORT", "18554")
internal_rtsp_port = os.environ.get("MEDIAMTX_INTERNAL_RTSP_PORT", "8554")

m = Marionette()  # (for Report only: no browser used)
r = Report()
if not shutil.which("ffmpeg"):
    print("NOTE : ffmpeg absent, l'endurance n'est PAS verifiee ici.")
    r.finish()
    sys.exit(0)

jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(method, path, data=None, headers=None):
    h = {"Origin": base}
    h.update(headers or {})
    req = urllib.request.Request(base + path, data=data, method=method, headers=h)
    try:
        with opener.open(req, timeout=60) as resp:
            return resp.status, resp.headers, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def post_json(path, body):
    status, _, raw = call("POST", path, json.dumps(body).encode(), {"Content-Type": "application/json"})
    return status, raw.decode()


def form(path, fields):
    data = urllib.parse.urlencode(fields).encode()
    status, _, raw = call("POST", path, data, {"Content-Type": "application/x-www-form-urlencoded"})
    return status, raw.decode()


status, _ = post_json("/api/auth/login", {"username": user, "password": password})
r.check("connexion administrateur", status == 200, str(status))

source = f"e2e-soak-src-{os.getpid()}"
camera = {
    "id": None, "name": f"E2E endurance {os.getpid()}", "location_id": None, "active": True, "priority": 1, "notes": "",
    "proto": "rtsp", "ip": "127.0.0.1", "port": int(internal_rtsp_port), "path": source, "rtsp_user": "", "rtsp_pass": "", "fps": 10,
    "rtsp_any_port": False, "rtsp_transport_tcp": False, "recording_mode": "continuous", "night_mode_api": "none", "web_port": 80,
    "web_api_key": "", "web_user": "", "web_pass": "", "onvif": False, "onvif_port": 80, "onvif_user": "", "onvif_pass": "", "ptz": False,
    "yolo_enabled": False, "yolo_classes": "", "yolo_min_confidence_pct": 50,
}
status, raw = post_json("/sf/cameras/save", {"input": camera})
r.check("camera creee en enregistrement continu", status == 200 and "Saved" in raw, raw[:120])
cam = json.loads(raw)["Saved"] if status == 200 and "Saved" in raw else {"id": 0, "uuid": "0" * 32}
uuid = cam["uuid"]


def days():
    """Les jours (locaux) couverts par l'essai : un essai de plus d'une heure peut passer minuit."""
    out = []
    t = datetime.datetime.now() - datetime.timedelta(seconds=60)
    end = datetime.datetime.now() + datetime.timedelta(seconds=duration + 600)
    while t.date() <= end.date():
        out.append(t.date().isoformat())
        t += datetime.timedelta(days=1)
    return sorted(set(out))


def all_items():
    out = {}
    for day in days():
        status, raw = form("/sf/recordings/list", {"uuid": uuid, "day": day})
        data = json.loads(raw) if status == 200 else None
        for item in (data or {}).get("items", []):
            out[item["id"]] = item
    return sorted(out.values(), key=lambda i: i["started_at"])


ff = None
try:
    t0 = time.time()
    ff = subprocess.Popen(
        ["ffmpeg", "-loglevel", "error", "-re", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=25",
         "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-g", "25", "-pix_fmt", "yuv420p",
         "-profile:v", "baseline", "-f", "rtsp", "-rtsp_transport", "tcp", f"rtsp://127.0.0.1:{rtsp_port}/{source}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    next_report = t0 + 300
    while time.time() - t0 < duration:
        time.sleep(min(10, max(1, duration - (time.time() - t0))))
        if ff.poll() is not None:
            r.check("le publieur ffmpeg est reste en vie", False, f"code {ff.returncode} apres {int(time.time() - t0)} s")
            break
        if time.time() >= next_report:
            next_report += 300
            items = all_items()
            print(f"  ... {int(time.time() - t0)} s : {len(items)} segments, {sum(1 for i in items if i['status'] == 'done')} archives", flush=True)
    published = time.time() - t0
    ff.terminate()
    ff.wait(10)
    # Lets the last segments finish (the video role's cool-down delay), get indexed then archived: we stop when nothing
    # new has appeared for 30 s and everything is archived.
    deadline, last_count, last_change = time.time() + 120, -1, time.time()
    while time.time() < deadline:
        items = all_items()
        if len(items) != last_count:
            last_count, last_change = len(items), time.time()
        if items and all(i["status"] == "done" for i in items) and time.time() - last_change > 30:
            break
        time.sleep(3)
    items = all_items()
    stamps = [datetime.datetime.fromisoformat(i["started_at"]) for i in items]
    gaps = [(b - a).total_seconds() for a, b in zip(stamps, stamps[1:])]
    worst = max(gaps) if gaps else 0.0
    total_ms = sum(i["duration_ms"] or 0 for i in items)
    total_bytes = sum(i["size_bytes"] or 0 for i in items)
    expected = published / segment_s
    print(f"  publie {published:.0f} s ; {len(items)} segments (attendus ~{expected:.0f}) ; plus grand ecart {worst:.1f} s ;"
          f" duree cumulee {total_ms / 1000:.0f} s ; {total_bytes / 1048576:.1f} Mio", flush=True)
    r.check("le nombre de segments correspond a la duree publiee", len(items) >= expected - 3, f"{len(items)} pour ~{expected:.0f}")
    r.check("aucun trou entre segments consecutifs (ecart max <= 1,5 segment)", worst <= segment_s * 1.5, f"{worst:.1f} s")
    r.check("aucun segment en erreur ni en attente", all(i["status"] == "done" for i in items),
            str([(i["id"], i["status"], i["error"]) for i in items if i["status"] != "done"][:5]))
    r.check("duree cumulee >= 97 % de la duree publiee", total_ms / 1000 >= 0.97 * published - segment_s, f"{total_ms / 1000:.0f} s sur {published:.0f} s")
    # Re-reading of all the segments from ReductStore (the local files are deleted after archiving).
    bad = []
    for i in items:
        status, headers, body = call("GET", f"/surveillance/recordings/{uuid}/{i['id']}/stream/")
        if status != 200 or body[4:8] != b"ftyp" or len(body) != i["size_bytes"]:
            bad.append((i["id"], status, len(body), i["size_bytes"]))
    r.check("tous les segments se relisent depuis ReductStore (MP4, taille indexee)", not bad, str(bad[:5]))
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
