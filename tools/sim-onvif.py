#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Camera ONVIF SIMULEE (E2E, R6 lot 3) : rejoue les reponses de `crates/dominet-surveillance/fixtures/onvif/<famille>/` et verifie
l'authentification WS-Security (digest) comme une vraie camera. Pendant de `dominet_surveillance::testing::onvif::FakeOnvif`.
Rien ne sort de la boucle locale.

    sim-onvif.py PORT FAMILLE UTILISATEUR MOT_DE_PASSE [JOURNAL]     (FAMILLE : thingino | vatilon | xiaomi | nvt)

Le PTZ (`ContinuousMove`, `Stop`, presets, `AbsoluteMove`) et l'imagerie (`IrCutFilter`, luminosite) gardent leur etat en memoire, comme
`FakeOnvif` ; chaque ordre recu est ajoute, en une ligne, au fichier JOURNAL (ex. `ContinuousMove 0.5 0 0`, `SetImagingSettings IrCutFilter=NIGHT`).
"""
import base64
import hashlib
import threading
import xml.etree.ElementTree as ET
import re
import sys
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

port, family, user, password = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
log_path = sys.argv[5] if len(sys.argv) > 5 else None
lock = threading.Lock()
STATE = {
    "velocity": None, "position": [0.0, 0.0, 0.0], "presets": [], "moving": 0, "next": 0,
    "imaging": [["BacklightCompensation/Mode", "OFF"], ["Brightness", "35"], ["IrCutFilter", "AUTO"], ["Sharpness", "50"]],
}
fixtures = Path(__file__).resolve().parent.parent / "crates/dominet-surveillance/fixtures/onvif" / family


def text_of(xml, local):
    m = re.search(rf"<(?:\w+:)?{local}[^>]*>([^<]*)</", xml)
    return m.group(1).strip() if m else None


def authenticated(xml):
    u, digest, nonce, created = (text_of(xml, t) for t in ("Username", "Password", "Nonce", "Created"))
    if not all((u, digest, nonce, created)) or u != user:
        return False
    expected = base64.b64encode(hashlib.sha1(base64.b64decode(nonce) + created.encode() + password.encode()).digest()).decode()
    try:
        when = datetime.strptime(created, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError:
        return False
    return expected == digest and abs(time.time() - when.timestamp()) <= 30


def local(tag):
    return tag.rsplit("}", 1)[-1]


def find(root, name):
    return next((e for e in root.iter() if local(e.tag) == name), None)


def axes(root, parent):
    p = find(root, parent)
    pt = find(p, "PanTilt") if p is not None else None
    zm = find(p, "Zoom") if p is not None else None
    num = lambda e, a: float(e.get(a)) if e is not None and e.get(a) else 0.0
    return num(pt, "x"), num(pt, "y"), num(zm, "x")


def envelope(body):
    pre = "env" if family == "nvt" else "SOAP-ENV"
    return (f'<?xml version="1.0" encoding="UTF-8"?><{pre}:Envelope xmlns:{pre}="http://www.w3.org/2003/05/soap-envelope" '
            'xmlns:tt="http://www.onvif.org/ver10/schema" xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl" '
            f'xmlns:timg="http://www.onvif.org/ver20/imaging/wsdl"><{pre}:Body>{body}</{pre}:Body></{pre}:Envelope>')


def log(line):
    if log_path:
        with open(log_path, "a") as f:
            f.write(line + "\n")


def dynamic(op, xml):
    """Reponse d'une operation PTZ ou d'imagerie (None si ce n'en est pas une)."""
    root = ET.fromstring(xml)
    st = STATE
    esc = lambda t: t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    if op == "ContinuousMove":
        v = axes(root, "Velocity")
        st["velocity"] = v
        log("ContinuousMove %g %g %g" % v)
        return "<tptz:ContinuousMoveResponse/>"
    if op == "Stop":
        st["velocity"] = None
        log("Stop")
        return "<tptz:StopResponse/>"
    if op == "GetStatus":
        moving = st["moving"] > 0 or st["velocity"] is not None
        st["moving"] = max(0, st["moving"] - 1)
        pan, tilt, zoom = (0.0, 0.0, 0.0) if family == "nvt" else st["position"]
        state = "MOVING" if moving else "IDLE"
        return (f'<tptz:GetStatusResponse><tptz:PTZStatus><tt:Position><tt:PanTilt x="{pan}" y="{tilt}" '
                'space="http://www.onvif.org/ver10/tptz/PanTiltSpaces/SphericalPositionSpace"/>'
                f'<tt:Zoom x="{zoom}"/></tt:Position><tt:MoveStatus><tt:PanTilt>{state}</tt:PanTilt><tt:Zoom>{state}</tt:Zoom>'
                '</tt:MoveStatus></tptz:PTZStatus></tptz:GetStatusResponse>')
    if op == "GetPresets":
        items = "".join(f'<tptz:Preset token="{t}"><tt:Name>{esc(n)}</tt:Name></tptz:Preset>' for t, n, _ in st["presets"])
        return f"<tptz:GetPresetsResponse>{items}</tptz:GetPresetsResponse>"
    if op == "SetPreset":
        name = (find(root, "PresetName").text or "").strip()
        st["next"] += 1
        token = f"preset_{st['next']}"
        st["presets"].append((token, name, list(st["position"])))
        log(f"SetPreset {name}")
        return f"<tptz:SetPresetResponse><tptz:PresetToken>{token}</tptz:PresetToken></tptz:SetPresetResponse>"
    if op == "RemovePreset":
        token = (find(root, "PresetToken").text or "").strip()
        st["presets"] = [p for p in st["presets"] if p[0] != token]
        log(f"RemovePreset {token}")
        return "<tptz:RemovePresetResponse/>"
    if op == "GotoPreset":
        token = (find(root, "PresetToken").text or "").strip()
        for t, _, at in st["presets"]:
            if t == token:
                st["position"] = list(at)
                st["moving"] = 2
        log(f"GotoPreset {token}")
        return "<tptz:GotoPresetResponse/>"
    if op == "AbsoluteMove":
        st["position"] = list(axes(root, "Position"))
        st["moving"] = 2
        log("AbsoluteMove %g %g %g" % tuple(st["position"]))
        return "<tptz:AbsoluteMoveResponse/>"
    if op == "GetImagingSettings":
        inner = ""
        for path, value in st["imaging"]:
            if "/" in path:
                parent, leaf = path.split("/")
                inner += f"<tt:{parent}><tt:{leaf}>{value}</tt:{leaf}></tt:{parent}>"
            else:
                inner += f"<tt:{path}>{value}</tt:{path}>"
        return f"<timg:GetImagingSettingsResponse><timg:ImagingSettings>{inner}</timg:ImagingSettings></timg:GetImagingSettingsResponse>"
    if op == "SetImagingSettings":
        settings = find(root, "ImagingSettings")
        out = []
        for child in settings:
            leaves = list(child)
            if not leaves:
                out.append([local(child.tag), (child.text or "").strip()])
            else:
                out += [[f"{local(child.tag)}/{local(leaf.tag)}", (leaf.text or "").strip()] for leaf in leaves]
        st["imaging"] = out
        changed = ",".join(f"{k}={v}" for k, v in out if k in ("IrCutFilter", "Brightness"))
        log(f"SetImagingSettings {changed}")
        return "<timg:SetImagingSettingsResponse/>"
    return None


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/soap+xml; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        xml = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode(errors="replace")
        body = re.search(r"<(?:\w+:)?Body[^>]*>\s*<(?:\w+:)?(\w+)", xml)
        op = body.group(1) if body else ""
        if op != "GetSystemDateAndTime" and not authenticated(xml):
            return self.reply(400, '<?xml version="1.0"?><e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"><e:Body><e:Fault><e:Reason><e:Text xml:lang="en">Sender not Authorized</e:Text></e:Reason></e:Fault></e:Body></e:Envelope>')
        with lock:
            body = dynamic(op, xml)
        if body is not None:
            return self.reply(200, envelope(body))
        path = fixtures / f"{op}.xml"
        if not path.is_file():
            return self.reply(400, '<?xml version="1.0"?><e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"><e:Body><e:Fault><e:Reason><e:Text xml:lang="en">Action not supported</e:Text></e:Reason></e:Fault></e:Body></e:Envelope>')
        now = datetime.now(timezone.utc)
        out = path.read_text()
        for key, value in {
            "HOST_ONLY": "127.0.0.1", "HOST": f"127.0.0.1:{port}", "PASSWORD": password,
            "YEAR": now.year, "MONTH": now.month, "DAY": now.day, "HOUR": now.hour, "MINUTE": now.minute, "SECOND": now.second,
        }.items():
            out = out.replace("{{" + key + "}}", str(value))
        self.reply(200, out)

    def log_message(self, fmt, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
