#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""API Open-Meteo **simulee** pour les E2E et les demonstrations : repond `GET /v1/forecast` par un releve fixe (18,2 °C) et compte les
requetes (`GET /__count`). N'accepte que la boucle locale.

Usage : tools/sim-open-meteo.py PORT
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

count = 0


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        global count
        if self.path.startswith("/__count"):
            body = json.dumps({"count": count}).encode()
        elif self.path.startswith("/v1/forecast"):
            count += 1
            body = json.dumps({"current": {
                "temperature_2m": 18.2, "relative_humidity_2m": 64, "apparent_temperature": 17.0, "precipitation": 0.0,
                "wind_speed_10m": 11.5, "wind_direction_10m": 200, "weather_code": 2, "uv_index": 3.0,
                "surface_pressure": 1013.0, "cloud_cover": 40}}).encode()
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):  # silencieux
        pass


HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
