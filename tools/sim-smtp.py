#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Serveur SMTP SIMULE (E2E, R5) : accepte tout, sans authentification ni STARTTLS, et ecrit chaque message recu comme une ligne JSON
(`from`, `to`, `data`) dans le fichier donne. Rien ne sort de la machine : c'est le pendant de `dominet_notify::testing::FakeSmtp`.

    sim-smtp.py PORT FICHIER
"""
import json
import socketserver
import sys


class Handler(socketserver.StreamRequestHandler):
    def reply(self, text):
        self.wfile.write(text.encode() + b"\r\n")
        self.wfile.flush()

    def handle(self):
        self.reply("220 sim.local ESMTP")
        sender, rcpts = "", []
        while True:
            line = self.rfile.readline()
            if not line:
                return
            cmd = line.decode(errors="replace").strip()
            up = cmd.upper()
            if up.startswith(("EHLO", "HELO")):
                self.reply("250-sim.local\r\n250 8BITMIME")
            elif up.startswith("MAIL FROM:"):
                sender, rcpts = cmd[10:].strip().strip("<>").split(">")[0], []
                self.reply("250 ok")
            elif up.startswith("RCPT TO:"):
                rcpts.append(cmd[8:].strip().strip("<>").split(">")[0])
                self.reply("250 ok")
            elif up == "DATA":
                self.reply("354 go ahead")
                data = []
                while True:
                    part = self.rfile.readline()
                    if not part or part in (b".\r\n", b".\n"):
                        break
                    data.append(part.decode(errors="replace").removeprefix("."))
                with open(sys.argv[2], "a") as f:
                    f.write(json.dumps({"from": sender, "to": rcpts, "data": "".join(data)}) + "\n")
                self.reply("250 queued")
            elif up == "QUIT":
                self.reply("221 bye")
                return
            else:
                self.reply("250 ok")


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: sim-smtp.py PORT FICHIER")
    with Server(("127.0.0.1", int(sys.argv[1])), Handler) as srv:
        srv.serve_forever()
