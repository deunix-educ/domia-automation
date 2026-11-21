#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Prometheus SIMULE (E2E, R13) : repond a `/api/v1/alerts` comme le vrai. Trois modes, changes par `GET /__mode/<mode>` :
`firing` (une alerte critique en cours et une en attente), `empty` (aucune alerte), `down` (erreur 503). Rien ne sort de la machine.

    sim-prometheus.py PORT
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

mode = "firing"
FIRING = [
    {"state": "firing", "activeAt": "2026-11-18T08:00:00Z",
     "labels": {"alertname": "SimMqttDown", "severity": "critical"},
     "annotations": {"summary": "MQTT simule hors ligne", "description": "essai E2E"}},
    {"state": "pending", "labels": {"alertname": "SimLent", "severity": "info"}, "annotations": {}},
]


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        global mode
        if self.path.startswith("/__mode/"):
            mode = self.path.rsplit("/", 1)[1]
            return self.reply(200, {"mode": mode})
        if self.path.startswith("/api/v1/alerts"):
            if mode == "down":
                return self.reply(503, {"status": "error"})
            alerts = FIRING if mode == "firing" else []
            return self.reply(200, {"status": "success", "data": {"alerts": alerts}})
        self.reply(404, {"status": "error"})

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
