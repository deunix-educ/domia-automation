#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Onduleur SolarMax **simule** pour les E2E et les demonstrations : serveur TCP local qui parle le protocole (valeurs fixes).
N'ecoute que la boucle locale ; ne se connecte a rien.

Usage : tools/sim-solarmax.py PORT
"""
import socketserver
import sys

VALUES = {
    "UDC": "0C0A", "IDC": "0130", "UL1": "08FC", "IL1": "0113", "PAC": "0640", "KDY": "00C8", "KT0": "1F40", "TKK": "002D",
    "ADR": "1", "TYP": "4E3E", "PIN": "0FA0", "SYS": "4E24,0", "SAL": "0",
}


def checksum(text):
    return format(sum(ord(c) for c in text), "04X")


def answer(inverter, keys):
    content = ";".join(f"{k}={VALUES[k]}" for k in keys)
    length = 0
    for _ in range(3):
        length = len(f"{inverter:02};FB;{length:X}|64:{content}|") + 4 + 2
    body = f"{inverter:02};FB;{length:X}|64:{content}|"
    return "{" + body + checksum(body) + "}"


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        buffer = ""
        while True:
            data = self.request.recv(1)
            if not data:
                return
            buffer += chr(data[0] & 0x7F)
            if not buffer.endswith("}"):
                continue
            request, buffer = buffer, ""
            try:
                inner = request[1:-1]
                content, total = inner[:-4], inner[-4:]
                assert total == checksum(content)
                header, body = content.split("|")[:2]
                inverter = int(header.split(";")[1])
                keys = body[3:].split(";")
                self.request.sendall(answer(inverter, keys).encode())
            except Exception:  # noqa: BLE001 - unreadable request: no answer, like a silent inverter
                pass


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


Server(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
