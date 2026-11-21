#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Mesure de charge de la video (R6 lot 8, critere « CPU / RAM mesures, nombre de cameras tenables »).

Lance, dans un espace a part (conteneurs jetables sur des ports decales 3xxxx, dossier de travail propre, rien de partage avec un systeme en service) :
PostgreSQL, MediaMTX (qui enregistre), ReductStore, puis `dominet serve`, et simule N cameras 1080p H.264 (un clip encode une fois, republie en
`-c copy` donc presque gratuit pour la machine). Pour chaque scenario et chaque N, mesure pendant `--seconds` : le CPU et la memoire de `dominet`, de
ses `ffmpeg` de decodage, de MediaMTX, de ReductStore et de PostgreSQL, et l'occupation totale de la machine.

Scenarios : `decode` (camera sans enregistrement ni detection : le role video decode a 2 images/s), `full` (mode mouvement, enregistrement
continu en fMP4 + archive + mouvement toutes les 30 s + YOLO), `stress` (comme `full` avec un mouvement evalue toutes les 5 s : YOLO ~6 fois plus souvent).

Usage : python3 tools/bench-video.py --bin ./dominet --site ./site --model ./yolov8n.onnx --bus ./bus.jpg --workdir /tmp/bench --counts 1,2,4,8
"""
import argparse
import http.cookiejar
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

PORTS = {"pg": 35432, "rtsp": 38554, "api": 39997, "reduct": 38383, "http": 38080}
PREFIX = "dominet-bench"
TICKS = os.sysconf("SC_CLK_TCK")


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout.strip()


def port_free(p):
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", p)) != 0


def ensure_image(name):
    if subprocess.run(["docker", "image", "inspect", name], capture_output=True).returncode != 0:
        print(f"  image {name} absente : docker pull", flush=True)
        run(["docker", "pull", name])


# ------------------------------------------------------------------------------------------------------------------------ mesure des processus
def proc_table():
    table = {}
    for pid in filter(str.isdigit, os.listdir("/proc")):
        try:
            with open(f"/proc/{pid}/stat") as f:
                raw = f.read()
            comm = raw[raw.index("(") + 1:raw.rindex(")")]
            rest = raw[raw.rindex(")") + 2:].split()
            with open(f"/proc/{pid}/cmdline", "rb") as f:
                cmd = f.read().replace(b"\0", b" ").decode(errors="replace")
            table[int(pid)] = {"comm": comm, "ppid": int(rest[1]), "ticks": int(rest[11]) + int(rest[12]), "cmd": cmd}
        except (FileNotFoundError, ProcessLookupError, ValueError, IndexError):
            continue
    return table


def rss_mb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except FileNotFoundError:
        pass
    return 0.0


def descendants(table, root):
    found, frontier = {root}, [root]
    while frontier:
        parent = frontier.pop()
        for pid, p in table.items():
            if p["ppid"] == parent and pid not in found:
                found.add(pid)
                frontier.append(pid)
    return found


def cpu_total():
    with open("/proc/stat") as f:
        v = [int(x) for x in f.readline().split()[1:]]
    return sum(v), v[3] + v[4]  # total, idle+iowait


def mem_used_mb():
    info = {}
    with open("/proc/meminfo") as f:
        for line in f:
            k, v = line.split(":")
            info[k] = int(v.split()[0])
    return (info["MemTotal"] - info["MemAvailable"]) / 1024


def docker_stats():
    out = run(["docker", "stats", "--no-stream", "--format", "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}"])
    stats = {}
    for line in out.splitlines():
        name, cpu, mem = line.split("\t")
        if not name.startswith(PREFIX):
            continue
        used = mem.split("/")[0].strip()
        factor = {"KiB": 1 / 1024, "MiB": 1, "GiB": 1024, "B": 1 / 1048576}
        unit = next(u for u in ("GiB", "MiB", "KiB", "B") if used.endswith(u))
        stats[name.removeprefix(PREFIX + "-")] = (float(cpu.rstrip("%")) / 100.0, float(used[:-len(unit)]) * factor[unit])
    return stats


# ------------------------------------------------------------------------------------------------------------------------ environnement
class Bench:
    def __init__(self, a):
        self.a = a
        self.work = os.path.abspath(a.workdir)
        self.procs = []
        self.server = None
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.base = f"http://127.0.0.1:{PORTS['http']}"
        self.token = "bench-token"

    def containers(self):
        for k in ("pg", "rtsp", "api", "reduct", "http"):
            if not port_free(PORTS[k]):
                sys.exit(f"le port {PORTS[k]} est deja pris : un autre essai tourne-t-il ?")
        for img in ("postgres:16-alpine", "reduct/store:v1.21.5", "bluenviron/mediamtx:1.21.3"):
            ensure_image(img)
        for d in ("rec", "reduct", "snapshots", "detections", "clip"):
            os.makedirs(os.path.join(self.work, d), exist_ok=True)
        conf = os.path.join(self.work, "mediamtx.yml")
        with open(conf, "w") as f:
            f.write("""logLevel: warn
api: true
apiAddress: :9997
authInternalUsers:
  - user: any
    pass:
    ips: []
    permissions:
      - {action: publish, path: ""}
      - {action: read, path: ""}
      - {action: api, path: ""}
rtsp: true
rtspAddress: :8554
rtmp: false
hls: false
webrtc: false
srt: false
record: no
recordFormat: fmp4
recordPath: /recordings/%path/%Y-%m-%d_%H-%M-%S-%f
recordPartDuration: 1s
recordSegmentDuration: 60s
recordDeleteAfter: 0s
paths:
  "~.*": {}
""")
        uid = f"{os.getuid()}:{os.getgid()}"
        run(["docker", "run", "-d", "--name", f"{PREFIX}-pg", "-e", "POSTGRES_USER=dominet", "-e", "POSTGRES_PASSWORD=bench",
             "-e", "POSTGRES_DB=dominet", "--tmpfs", "/var/lib/postgresql/data", "-p", f"127.0.0.1:{PORTS['pg']}:5432", "postgres:16-alpine"])
        run(["docker", "run", "-d", "--name", f"{PREFIX}-mediamtx", "--user", uid, "-v", f"{conf}:/mediamtx.yml:ro",
             "-v", f"{self.work}/rec:/recordings", "-p", f"127.0.0.1:{PORTS['rtsp']}:8554", "-p", f"127.0.0.1:{PORTS['api']}:9997",
             "bluenviron/mediamtx:1.21.3"])
        run(["docker", "run", "-d", "--name", f"{PREFIX}-reduct", "--user", uid, "-e", f"RS_API_TOKEN={self.token}",
             "-v", f"{self.work}/reduct:/data", "-p", f"127.0.0.1:{PORTS['reduct']}:8383", "reduct/store:v1.21.5"])
        for _ in range(60):
            if subprocess.run(["docker", "exec", f"{PREFIX}-pg", "pg_isready", "-U", "dominet"], capture_output=True).returncode == 0:
                break
            time.sleep(1)
        time.sleep(2)

    def env(self, motion_interval):
        e = dict(os.environ)
        e.update({
            "DATABASE_URL": f"postgres://dominet:bench@127.0.0.1:{PORTS['pg']}/dominet", "COOKIE_SECURE": "false",
            "HTTP_ADDR": f"127.0.0.1:{PORTS['http']}", "DOMINET_MASTER_KEY": self.master_key,
            "MEDIAMTX_API_URL": f"http://127.0.0.1:{PORTS['api']}", "MEDIAMTX_RTSP_URL": f"rtsp://127.0.0.1:{PORTS['rtsp']}",
            "SNAPSHOTS_DIR": f"{self.work}/snapshots", "RECORDINGS_DIR": f"{self.work}/rec", "RECORDINGS_SEGMENT_DURATION": "60",
            "REDUCTSTORE_URL": f"http://127.0.0.1:{PORTS['reduct']}", "REDUCTSTORE_TOKEN": self.token,
            "YOLO_MODEL_PATH": os.path.abspath(self.a.model), "DETECTIONS_DIR": f"{self.work}/detections",
            "MOTION_INTERVAL_SECS": str(motion_interval), "RUST_LOG": os.environ.get("RUST_LOG", "warn"), "SITE_ROOT": os.path.abspath(self.a.site),
        })
        return e

    def setup_app(self):
        self.master_key = run([self.a.bin, "gen-master-key"], env=dict(os.environ)).splitlines()[-1]
        env = self.env(30)
        run([self.a.bin, "migrate"], env=env)
        run([self.a.bin, "seed"], env=env)
        self.password = "bench-" + os.urandom(6).hex()
        subprocess.run([self.a.bin, "createsuperuser", "--username", "bench"], env={**env, "DOMINET_PASSWORD": self.password},
                       check=True, capture_output=True)

    def start_server(self, motion_interval):
        env = self.env(motion_interval)
        cmd = [self.a.bin, "serve"]
        if self.a.nice:
            cmd = ["nice", "-n", str(self.a.nice)] + cmd
        self.server = subprocess.Popen(cmd, env=env, stdout=open(os.path.join(self.work, "server.log"), "w"), stderr=subprocess.STDOUT)
        for _ in range(100):
            try:
                urllib.request.urlopen(self.base + "/healthz", timeout=2)
                break
            except Exception:
                time.sleep(0.3)
        self.login()

    def stop_server(self):
        if self.server and self.server.poll() is None:
            self.server.terminate()
            try:
                self.server.wait(15)
            except subprocess.TimeoutExpired:
                self.server.kill()
        self.server = None

    def call(self, path, data, ctype):
        req = urllib.request.Request(self.base + path, data=data, method="POST", headers={"Content-Type": ctype, "Origin": self.base})
        try:
            with self.opener.open(req, timeout=30) as r:
                return r.status, r.read().decode()
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode()

    def login(self):
        s, _ = self.call("/api/auth/login", json.dumps({"username": "bench", "password": self.password}).encode(), "application/json")
        assert s == 200, s

    def form(self, path, **fields):
        data = urllib.parse.urlencode({k: str(v).lower() if isinstance(v, bool) else v for k, v in fields.items()}).encode()
        return self.call(path, data, "application/x-www-form-urlencoded")

    def make_clip(self):
        os.makedirs(os.path.join(self.work, "clip"), exist_ok=True)
        clip = os.path.join(self.work, "clip", f"clip-{self.a.resolution}-{self.a.fps}.mp4")
        if os.path.exists(clip):
            return clip
        w, h = self.a.resolution.split("x")
        run(["ffmpeg", "-loglevel", "error", "-y", "-loop", "1", "-framerate", str(self.a.fps), "-i", self.a.bus, "-t", "20",
             "-vf", f"scale={w}:{h},eq=brightness='0.25*sin(2*PI*t*0.6)':eval=frame,format=yuv420p",
             "-c:v", "libx264", "-preset", "medium", "-b:v", "3M", "-maxrate", "3M", "-bufsize", "6M", "-g", str(self.a.fps * 2),
             "-profile:v", "main", clip])
        return clip

    def camera(self, i, mode, yolo):
        return {
            "id": None, "name": f"bench{i}", "location_id": None, "active": True, "priority": 1, "notes": "", "proto": "rtsp",
            "ip": "127.0.0.1", "port": 8554, "path": f"src{i}", "rtsp_user": "", "rtsp_pass": "", "fps": self.a.fps,
            "rtsp_any_port": False, "rtsp_transport_tcp": False, "recording_mode": mode, "night_mode_api": "none", "web_port": 80,
            "web_api_key": "", "web_user": "", "web_pass": "", "onvif": False, "onvif_port": 80, "onvif_user": "", "onvif_pass": "",
            "ptz": False, "yolo_enabled": yolo, "yolo_classes": "bus", "yolo_min_confidence_pct": 30,
        }

    def cleanup(self):
        self.stop_server()
        for p in self.procs:
            if p.poll() is None:
                p.terminate()
        for name in ("pg", "mediamtx", "reduct"):
            subprocess.run(["docker", "rm", "-f", f"{PREFIX}-{name}"], capture_output=True)

    # -------------------------------------------------------------------------------------------------------------------- un essai
    def trial(self, scenario, n, clip):
        mode, yolo, interval = {"decode": ("none", False, 30), "full": ("motion", True, 30), "stress": ("motion", True, 5)}[scenario]
        self.start_server(interval)
        self.form("/sf/detector/set", enabled=yolo, save_frames=False)
        uuids = []
        for i in range(n):
            s, text = self.call("/sf/cameras/save", json.dumps({"input": self.camera(i, mode, yolo)}).encode(), "application/json")
            assert s == 200 and "Saved" in text, text[:200]
            uuids.append(json.loads(text)["Saved"])
        pubs = []
        for i in range(n):
            cmd = ["ffmpeg", "-loglevel", "error", "-re", "-stream_loop", "-1", "-i", clip, "-c", "copy", "-f", "rtsp",
                   "-rtsp_transport", "tcp", f"rtsp://127.0.0.1:{PORTS['rtsp']}/src{i}"]
            if self.a.nice:
                cmd = ["nice", "-n", "19"] + cmd
            pubs.append(subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        try:
            # Attend que chaque camera ait sa derniere image (preuve que le decodage tourne).
            ok = False
            for _ in range(120):
                have = sum(os.path.exists(f"{self.work}/snapshots/latest/{c['uuid']}.jpg") for c in uuids)
                if have == n:
                    ok = True
                    break
                time.sleep(1)
            if not ok:
                print(f"  ATTENTION : {have}/{n} cameras ont une image", flush=True)
            time.sleep(self.a.warmup)
            samples = self.sample(self.a.seconds, [p.pid for p in pubs])
            events = self.events(uuids)
            fallbacks = self.count_log("VAAPI inutilisable")
            return {"scenario": scenario, "cameras": n, **summarise(samples), **events, "images_ok": ok, "vaapi_fallbacks": fallbacks}
        finally:
            for p in pubs:
                p.terminate()
            for p in pubs:
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
            for c in uuids:
                self.form("/sf/cameras/delete", id=c["id"])
            self.stop_server()
            time.sleep(2)

    def count_log(self, needle):
        try:
            with open(os.path.join(self.work, "server.log"), errors="replace") as f:
                return sum(needle in line for line in f)
        except FileNotFoundError:
            return 0

    def events(self, uuids):
        motion = detection = 0
        s, text = self.form("/sf/events/list", camera="")
        if s == 200:
            for e in json.loads(text):
                motion += e["kind"] == "motion"
                detection += e["kind"] == "detection"
        return {"motion_events": motion, "detection_events": detection}

    def sample(self, seconds, publisher_pids):
        samples = []
        prev = None
        end = time.time() + seconds
        while time.time() < end:
            table = proc_table()
            tree = descendants(table, self.server.pid)
            ff = {p: table[p]["ticks"] for p in tree if table.get(p, {}).get("comm") == "ffmpeg"}
            pubs = {p: table[p]["ticks"] for p in publisher_pids if p in table}
            # Les `ffmpeg` du role video et les cameras simulees changent parfois de pid (relance) : on additionne les ecarts par pid.
            now = time.time()
            total, idle = cpu_total()
            cur = {"t": now, "total": total, "idle": idle, "dominet": table[self.server.pid]["ticks"], "ff": ff, "pubs": pubs}
            dock = docker_stats()
            if prev:
                dt = cur["t"] - prev["t"]
                delta = lambda a, b: sum(max(0, v - a[k]) for k, v in b.items() if k in a)  # noqa: E731
                samples.append({
                    "dominet_cores": (cur["dominet"] - prev["dominet"]) / TICKS / dt,
                    "ffmpeg_cores": delta(prev["ff"], cur["ff"]) / TICKS / dt,
                    "publishers_cores": delta(prev["pubs"], cur["pubs"]) / TICKS / dt,
                    "mediamtx_cores": dock.get("mediamtx", (0, 0))[0], "reduct_cores": dock.get("reduct", (0, 0))[0],
                    "postgres_cores": dock.get("pg", (0, 0))[0],
                    "busy_pct": 100.0 * (1 - (cur["idle"] - prev["idle"]) / max(1, cur["total"] - prev["total"])),
                    "dominet_mb": rss_mb(self.server.pid), "ffmpeg_mb": sum(rss_mb(p) for p in ff), "ffmpeg_count": len(ff),
                    "mediamtx_mb": dock.get("mediamtx", (0, 0))[1], "reduct_mb": dock.get("reduct", (0, 0))[1],
                    "postgres_mb": dock.get("pg", (0, 0))[1], "machine_used_mb": mem_used_mb(),
                })
            prev = cur
            time.sleep(max(0.0, 5 - (time.time() - now)))
        return samples


def summarise(samples):
    keys = [k for k in samples[0] if k != "ffmpeg_count"] if samples else []
    out = {}
    for k in keys:
        vals = [s[k] for s in samples]
        out[k] = round(sum(vals) / len(vals), 2)
        out[k + "_max"] = round(max(vals), 2)
    out["ffmpeg_count"] = samples[-1]["ffmpeg_count"] if samples else 0
    out["video_cores"] = round(out.get("dominet_cores", 0) + out.get("ffmpeg_cores", 0) + out.get("mediamtx_cores", 0)
                               + out.get("reduct_cores", 0) + out.get("postgres_cores", 0), 2)
    out["video_mb"] = round(out.get("dominet_mb", 0) + out.get("ffmpeg_mb", 0) + out.get("mediamtx_mb", 0) + out.get("reduct_mb", 0)
                            + out.get("postgres_mb", 0))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    ap.add_argument("--site", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--bus", required=True)
    ap.add_argument("--workdir", required=True)
    ap.add_argument("--counts", default="1,2,4,8")
    ap.add_argument("--scenarios", default="decode,full,stress")
    ap.add_argument("--seconds", type=int, default=90)
    ap.add_argument("--warmup", type=int, default=20)
    ap.add_argument("--resolution", default="1920x1080")
    ap.add_argument("--fps", type=int, default=15)
    ap.add_argument("--nice", type=int, default=0, help="priorite reduite pour tout l'essai (machine partagee)")
    ap.add_argument("--out", default="bench.json")
    a = ap.parse_args()
    b = Bench(a)
    results = []
    try:
        os.makedirs(b.work, exist_ok=True)
        clip = b.make_clip()
        print(f"clip : {clip}", flush=True)
        b.containers()
        b.setup_app()
        for scenario in a.scenarios.split(","):
            for n in map(int, a.counts.split(",")):
                print(f"== {scenario} x {n} cameras", flush=True)
                r = b.trial(scenario, n, clip)
                results.append(r)
                print(f"   video : {r['video_cores']} coeurs, {r['video_mb']} Mo ; machine {r['busy_pct']} % ; "
                      f"mouvements {r['motion_events']}, detections {r['detection_events']}, replis logiciels {r['vaapi_fallbacks']}", flush=True)
                shutil.copy(os.path.join(b.work, "server.log"), os.path.join(b.work, f"server-{scenario}-{n}.log"))
                with open(a.out, "w") as f:
                    json.dump({"machine": os.uname().nodename, "cpus": os.cpu_count(), "resolution": a.resolution, "fps": a.fps,
                               "seconds": a.seconds, "results": results}, f, indent=1)
    finally:
        b.cleanup()
    print("\n| scenario | cameras | coeurs (video) | Mo (video) | dominet | ffmpeg | MediaMTX | ReductStore | cameras simulees | machine | replis |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for r in results:
        print(f"| {r['scenario']} | {r['cameras']} | {r['video_cores']} | {r['video_mb']} | {r['dominet_cores']} | {r['ffmpeg_cores']} | "
              f"{r['mediamtx_cores']} | {r['reduct_cores']} | {r['publishers_cores']} | {r['busy_pct']} % | {r['vaapi_fallbacks']} |")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(1))
    main()
