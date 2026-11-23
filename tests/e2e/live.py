#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E du live (R6 lot 4) : un VRAI flux H.264 (ffmpeg -> MediaMTX de dev) regarde par un navigateur qui decode le H.264 (Chromium sans
ecran, pilote par le protocole DevTools). Verifie : la page passe a l'etat « live » avec la bonne resolution, les statistiques s'affichent,
la perte du flux est signalee puis **le retour du flux est retrouve tout seul** ; et, dans Firefox (celui de Debian ne decode pas le
H.264 en WebRTC), le message explicite au lieu d'un echec muet.

Prerequis : chromium (ou google-chrome), ffmpeg avec libx264, python3 `websockets`, MediaMTX de la stack de dev. Sinon : rien n'est verifie.
Usage : python3 tests/e2e/live.py url utilisateur_admin mot_de_passe          (variables : MEDIAMTX_API_URL, MEDIAMTX_RTSP_PORT, MEDIAMTX_INTERNAL_RTSP_PORT)
"""
import http.cookiejar
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: live.py url utilisateur_admin mot_de_passe")
url, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
base = url.rstrip("/")
api = os.environ.get("MEDIAMTX_API_URL", "http://127.0.0.1:19997")
rtsp_port = os.environ.get("MEDIAMTX_RTSP_PORT", "18554")  # seen from the machine (publication by ffmpeg)
internal_rtsp_port = os.environ.get("MEDIAMTX_INTERNAL_RTSP_PORT", "8554")  # seen from MediaMTX itself (the camera pulls the test stream)
whep_domain = os.environ.get("MEDIAMTX_WHEP_DOMAIN", "127.0.0.1:18889")

m = Marionette()
r = Report()

browser = next((b for b in ("chromium", "chromium-browser", "google-chrome", "brave-browser") if shutil.which(b)), None)
try:
    from websockets.sync.client import connect as ws_connect
except ImportError:  # pragma: no cover
    ws_connect = None
have_chrome = bool(browser and shutil.which("ffmpeg") and ws_connect)
if not have_chrome:
    print("NOTE : chromium, ffmpeg ou python3-websockets absent : le live H.264 n'est PAS verifie ici.")

# ---------------------------------------------------------------------------------------------------------------- HTTP (admin session)
jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, data=data, method=method,
                                 headers={"Content-Type": "application/json", "Origin": base})
    try:
        with opener.open(req, timeout=15) as resp:
            return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


status, _ = call("POST", "/api/auth/login", {"username": user, "password": password})
r.check("connexion administrateur", status == 200, str(status))

# Dev MediaMTX: the browser calls it on this domain (interface setting, as in production).
status, _ = call("PUT", "/api/settings/MEDIAMTX_WEBRTC_DOMAIN", {"value": whep_domain})
r.check("reglage du domaine WebRTC", status == 200, str(status))

source = f"e2e-src-{os.getpid()}"
camera = {
    "id": None, "name": f"E2E live {os.getpid()}", "location_id": None, "active": True, "priority": 1, "notes": "",
    "proto": "rtsp", "ip": "127.0.0.1", "port": int(internal_rtsp_port), "path": source, "rtsp_user": "", "rtsp_pass": "", "fps": 10,
    "rtsp_any_port": False, "rtsp_transport_tcp": False, "recording_mode": "none", "night_mode_api": "none", "web_port": 80,
    "web_api_key": "", "web_user": "", "web_pass": "", "onvif": False, "onvif_port": 80, "onvif_user": "", "onvif_pass": "", "ptz": False,
    "yolo_enabled": False, "yolo_classes": "", "yolo_min_confidence_pct": 50,
}
status, text = call("POST", "/sf/cameras/save", {"input": camera})
r.check("camera creee (MediaMTX la tire depuis le flux de test)", status == 200 and "Saved" in text, text[:120])
cam = json.loads(text)["Saved"] if status == 200 and "Saved" in text else {"id": 0, "uuid": "0" * 32}
uuid = cam["uuid"]

procs = []


def start_ffmpeg():
    p = subprocess.Popen(
        ["ffmpeg", "-loglevel", "error", "-re", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=25",
         "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-g", "25", "-pix_fmt", "yuv420p",
         "-profile:v", "baseline", "-f", "rtsp", "-rtsp_transport", "tcp", f"rtsp://127.0.0.1:{rtsp_port}/{source}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(p)
    return p


def stop(p):
    if p and p.poll() is None:
        p.terminate()
        try:
            p.wait(5)
        except subprocess.TimeoutExpired:
            p.kill()


def mediamtx_ready(path, timeout=20):
    end = time.time() + timeout
    while time.time() < end:
        try:
            with urllib.request.urlopen(f"{api}/v3/paths/get/{path}", timeout=3) as resp:
                if json.load(resp).get("ready"):
                    return True
        except Exception:
            pass
        time.sleep(0.5)
    return False


class Cdp:
    """Client minimal du protocole DevTools de Chromium."""

    def __init__(self, ws_url):
        self.ws = ws_connect(ws_url, max_size=None, open_timeout=15)
        self.n = 0

    def call(self, method, **params):
        self.n += 1
        self.ws.send(json.dumps({"id": self.n, "method": method, "params": params}))
        while True:
            msg = json.loads(self.ws.recv(timeout=60))
            if msg.get("id") == self.n:
                if "error" in msg:
                    raise RuntimeError(msg["error"])
                return msg["result"]

    def eval(self, expression):
        res = self.call("Runtime.evaluate", expression=expression, returnByValue=True, awaitPromise=True)
        return res.get("result", {}).get("value")

    def wait(self, expression, timeout):
        end = time.time() + timeout
        while time.time() < end:
            try:
                if self.eval(expression):
                    return True
            except Exception:
                pass
            time.sleep(0.5)
        return False


chrome = None
profile = tempfile.mkdtemp(prefix="dominet-live-")
try:
    if have_chrome:
        # ---------------------------------------------------------------------------------------------- test stream and capable browser
        ff = start_ffmpeg()
        r.check("flux H.264 de test publie dans MediaMTX", mediamtx_ready(source), "ffmpeg ou MediaMTX")
        r.check("MediaMTX relaie la camera sous son identifiant public", mediamtx_ready(uuid, 25))

    # ---------------------------------------------------------------------------------------------- Firefox: explicit message without H.264
    m.goto(url)
    m.cmd("WebDriver:DeleteAllCookies")
    m.js_async(
        "const cb = arguments[arguments.length-1];"
        "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
        f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => cb(x.status));"
    )
    m.goto(f"{url}surveillance/live/{uuid}")
    r.check("page live rendue (Firefox)", m.wait_for("document.querySelector('.vc-live') !== null", 15))
    firefox_h264 = m.js("const c = RTCRtpReceiver.getCapabilities('video'); return !!c && c.codecs.some(k => /H264/i.test(k.mimeType));")
    if not firefox_h264:
        r.check("Firefox sans H.264 : message explicite, pas d'echec muet",
                m.wait_for("document.querySelector('.vc-live')?.dataset.liveState === 'no-h264'", 20),
                m.text(".vc-live-message") or "")
        r.check("le message dit quoi faire", "H.264" in (m.text(".vc-live-message") or ""))
        if have_chrome:
            # Fallback: a still image refreshed every second, taken from the stream decoded by the video role (ADR 0048).
            r.check("repli : une image apparait pour le navigateur sans H.264",
                    m.wait_for("(() => { const i = document.querySelector('.vc-live-fallback'); return !!i && i.complete && i.naturalWidth === 640; })()", 40),
                    str(m.js("const i = document.querySelector('.vc-live-fallback'); return i ? [i.complete, i.naturalWidth, i.src] : null")))
            first = m.js("return document.querySelector('.vc-live-fallback').src")
            r.check("... rafraichie : l'adresse change", m.wait_for(f"document.querySelector('.vc-live-fallback').src !== {first!r}", 10))
    else:
        print("NOTE : ce Firefox decode le H.264 : le message d'absence de H.264 n'est pas verifiable avec lui.")


    if have_chrome:
        port = 19222 + os.getpid() % 500
        chrome = subprocess.Popen(
            [browser, "--headless=new", f"--remote-debugging-port={port}", f"--user-data-dir={profile}", "--disable-gpu",
             "--autoplay-policy=no-user-gesture-required", "--disable-features=WebRtcHideLocalIpsWithMdns",
             "--no-first-run", "--no-default-browser-check", "about:blank"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        ws_url = None
        for _ in range(120):  # a first cold start can exceed 30 s on a loaded machine
            try:
                req = urllib.request.Request(f"http://127.0.0.1:{port}/json/new?about:blank", method="PUT")
                with urllib.request.urlopen(req, timeout=2) as resp:
                    ws_url = json.load(resp)["webSocketDebuggerUrl"]
                break
            except Exception:
                time.sleep(0.5)
        r.check("navigateur Chromium sans ecran demarre", ws_url is not None)
        cdp = Cdp(ws_url)
        cdp.call("Page.enable")
        cdp.call("Page.navigate", url=url + "login")
        cdp.wait("document.readyState === 'complete'", 15)
        status = cdp.eval(
            "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
            f"body: JSON.stringify({{username:{user!r}, password:{password!r}}})}}).then(x => x.status)")
        r.check("connexion dans Chromium", status == 200, str(status))
        has_h264 = cdp.eval("RTCRtpReceiver.getCapabilities('video').codecs.some(k => /H264/i.test(k.mimeType))")
        r.check("ce Chromium annonce le H.264 en WebRTC", has_h264 is True)

        cdp.call("Page.navigate", url=f"{url}surveillance/live/{uuid}")
        r.check("page live rendue (Chromium)", cdp.wait("document.querySelector('.vc-live') !== null", 20))
        # The development WASM weighs ~48 MB: hydration can take a few tens of seconds.
        live = cdp.wait("document.querySelector('.vc-live')?.dataset.liveState === 'live'", 90)
        state = cdp.eval("document.querySelector('.vc-live')?.dataset.liveState")
        r.check("le flux H.264 passe a l'etat « live »", live, f"etat : {state} ; {cdp.eval('document.querySelector(\".vc-live-message\")?.textContent')}")
        width = cdp.eval("document.querySelector('video').videoWidth")
        r.check("la video a la resolution du flux (640x360)", width == 640 and cdp.eval("document.querySelector('video').videoHeight") == 360, str(width))
        r.check("la video avance (temps de lecture croissant)",
                cdp.wait("document.querySelector('video').currentTime > 1", 15), str(cdp.eval("document.querySelector('video').currentTime")))
        r.check("les statistiques s'affichent", cdp.wait("/640×360/.test(document.querySelector('.vc-live-stats')?.textContent || '')", 15),
                str(cdp.eval("document.querySelector('.vc-live-stats')?.textContent")))

        # ------------------------------------------------------------------------------------------------- decoded images: last image, snapshots
        def fetch(path):
            try:
                with opener.open(urllib.request.Request(base + path, headers={"Origin": base}), timeout=15) as resp:
                    return resp.status, resp.read()
            except urllib.error.HTTPError as e:
                return e.code, b""

        def jpeg_size(data):
            i = 2
            while i + 9 < len(data) and data[i] == 0xFF:
                if data[i + 1] in (0xC0, 0xC1, 0xC2):
                    return int.from_bytes(data[i + 7:i + 9], "big"), int.from_bytes(data[i + 5:i + 7], "big")
                i += 2 + int.from_bytes(data[i + 2:i + 4], "big")
            return None

        snap = f"/surveillance/camera/{uuid}/snapshot/"
        got = (0, b"")
        for _ in range(60):
            got = fetch(snap)
            if got[0] == 200:
                break
            time.sleep(0.5)
        r.check("derniere image de la camera (role video)", got[0] == 200 and got[1].startswith(b"\xff\xd8") and got[1].endswith(b"\xff\xd9"), str(got[0]))
        r.check("... a la resolution du flux (640x360, jamais agrandie)", jpeg_size(got[1]) == (640, 360), str(jpeg_size(got[1])))
        forced = fetch(snap + "?force=1")
        r.check("?force=1 : une image fraiche", forced[0] == 200 and forced[1].startswith(b"\xff\xd8"))
        req = urllib.request.Request(base + "/sf/snapshots/capture", data=f"uuid={uuid}".encode(), method="POST",
                                     headers={"Content-Type": "application/x-www-form-urlencoded", "Origin": base})
        with opener.open(req, timeout=20) as resp:
            captured = json.loads(resp.read().decode())
        r.check("capture : snapshot inscrit dans la galerie", "Saved" in captured, str(captured)[:120])
        snap_id = captured["Saved"]["id"]
        status, file_bytes = fetch(f"/surveillance/snapshots/{snap_id}/file/")
        r.check("... et son fichier est servi", status == 200 and jpeg_size(file_bytes) == (640, 360))
        cdp.call("Page.navigate", url=url + "surveillance/snapshots")
        r.check("la galerie l'affiche", cdp.wait(f"document.querySelector('[data-snapshot=\"{snap_id}\"] img') !== null", 60))
        req = urllib.request.Request(base + "/sf/snapshots/delete", data=f"id={snap_id}".encode(), method="POST",
                                     headers={"Content-Type": "application/x-www-form-urlencoded", "Origin": base})
        with opener.open(req, timeout=20) as resp:
            deleted = resp.read().decode()
        r.check("suppression d'un snapshot", "Deleted" in deleted and fetch(f"/surveillance/snapshots/{snap_id}/file/")[0] == 404, deleted[:60])
        cdp.call("Page.navigate", url=f"{url}surveillance/live/{uuid}")
        cdp.wait("document.querySelector('.vc-live')?.dataset.liveState === 'live'", 90)

        # --------------------------------------------------------------------------------------------- stream loss, then automatic return
        stop(ff)
        r.check("flux perdu : le lecteur le dit (pas de video figee silencieuse)",
                cdp.wait("['reconnecting','no-stream','connecting'].includes(document.querySelector('.vc-live')?.dataset.liveState)", 40),
                str(cdp.eval("document.querySelector('.vc-live')?.dataset.liveState")))
        ff = start_ffmpeg()
        r.check("le flux revient", mediamtx_ready(source))
        r.check("le lecteur se reconnecte tout seul", cdp.wait("document.querySelector('.vc-live')?.dataset.liveState === 'live'", 90),
                str(cdp.eval("document.querySelector('.vc-live')?.dataset.liveState")))

        # ------------------------------------------------------------------------------------------------------ clean session close
        before = None
        try:
            with urllib.request.urlopen(f"{api}/v3/webrtcsessions/list", timeout=5) as resp:
                before = json.load(resp).get("itemCount")
        except Exception:
            pass
        cdp.call("Page.navigate", url=url + "login")
        time.sleep(4)
        try:
            with urllib.request.urlopen(f"{api}/v3/webrtcsessions/list", timeout=5) as resp:
                after = json.load(resp).get("itemCount")
            r.check("quitter la page ferme la session WebRTC chez MediaMTX", before is not None and after is not None and after < before,
                    f"avant {before}, apres {after}")
        except Exception as e:  # noqa: BLE001
            r.check("quitter la page ferme la session WebRTC chez MediaMTX", False, str(e))
finally:
    for p in procs:
        stop(p)
    if chrome:
        stop(chrome)
    shutil.rmtree(profile, ignore_errors=True)
    if cam["id"]:
        req = urllib.request.Request(base + "/sf/cameras/delete", data=f"id={cam['id']}".encode(), method="POST",
                                     headers={"Content-Type": "application/x-www-form-urlencoded", "Origin": base})
        try:
            opener.open(req, timeout=15).read()
        except Exception:
            pass

r.finish()
