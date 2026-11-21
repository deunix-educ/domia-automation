#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""API Free Mobile SIMULEE (demo, R5) : `GET /sendmsg?user=&pass=&msg=` repond 200 et ecrit la requete (sans la cle) comme une ligne JSON
dans le fichier donne. Rien ne sort de la machine : c'est le pendant de `dominet_notify::testing::FakeFreeMobile`.

    sim-free-mobile.py PORT FICHIER
"""
import json
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        u = urllib.parse.urlsplit(self.path)
        q = urllib.parse.parse_qs(u.query)
        if u.path.rstrip("/") != "/sendmsg":
            self.send_response(404)
            self.end_headers()
            return
        with open(sys.argv[2], "a") as f:
            f.write(json.dumps({"user": q.get("user", [""])[0], "msg": q.get("msg", [""])[0]}, ensure_ascii=False) + "\n")
        self.send_response(200)
        self.end_headers()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: sim-free-mobile.py PORT FICHIER")
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
