#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Telephone Android SIMULE qui reste connecte (demo, R5) : meme protocole que l'application Kotlin (voir tools/sim_android.py).

    sim-phone.py URL JETON DOSSIER

Il accuse chaque `send_sms` recu (reussite par defaut), journalise ce qu'il recoit dans DOSSIER/phone.log et lit DOSSIER/phone.cmd,
ajoute des lignes :  `sms NUMERO texte`,  `call ETAT NUMERO`,  `ack ok|fail|none`.
"""
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sim_android import Phone  # noqa: E402

url, token, folder = sys.argv[1], sys.argv[2], sys.argv[3]
cmd_file, log_file = os.path.join(folder, "phone.cmd"), os.path.join(folder, "phone.log")
open(cmd_file, "a").close()


def log(text):
    with open(log_file, "a") as f:
        f.write(time.strftime("%H:%M:%S ") + text + "\n")


phone = Phone(url, token, ping_interval=20)
phone.connect()
log("connecte")
seen_frames, offset = 0, os.path.getsize(cmd_file)
while True:
    time.sleep(0.5)
    for f in phone.frames[seen_frames:]:
        log("recu " + str({k: v for k, v in f.items()}))
    seen_frames = len(phone.frames)
    with open(cmd_file) as c:
        c.seek(offset)
        lines = c.read().splitlines()
        offset = c.tell()
    for line in lines:
        parts = line.split(" ", 2)
        if parts[0] == "sms" and len(parts) == 3:
            phone.sms(parts[1], parts[2], contact="Demo")
        elif parts[0] == "call" and len(parts) >= 3:
            phone.call(parts[1], parts[2], contact="Demo")
        elif parts[0] == "ack" and len(parts) >= 2:
            phone.ack = None if parts[1] == "none" else parts[1]
        log("commande " + line)
    if not phone.connected:
        log("deconnecte")
        sys.exit(0)
