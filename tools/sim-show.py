#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Affiche ce qu'ont recu le SMTP simule (tools/sim-smtp.py) ou le Free Mobile simule (tools/sim-free-mobile.py) : sim-show.py FICHIER."""
import json
import sys

n = 0
try:
    lines = [x for x in open(sys.argv[1]) if x.strip()]
except FileNotFoundError:
    lines = []
for line in lines:
    d = json.loads(line)
    n += 1
    if "data" in d:
        subject = next((x for x in d["data"].splitlines() if x.lower().startswith("subject:")), "")
        print(f"{d['from']} -> {', '.join(d['to'])} | {subject}")
    else:
        print(f"utilisateur {d['user']} : {d['msg']}")
if n == 0:
    print("(rien recu)")
