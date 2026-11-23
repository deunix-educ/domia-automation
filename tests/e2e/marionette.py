# SPDX-License-Identifier: AGPL-3.0-or-later
"""Client minimal du protocole Marionette de Firefox (aucune dependance Python externe).

Firefox se lance avec : firefox --headless --marionette --profile <dossier> -no-remote about:blank
"""
import json
import socket
import time


class Marionette:
    def __init__(self, port=2828, timeout=30):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        self.msg_id = 0
        self._read()  # greeting message
        self.cmd("WebDriver:NewSession", {"capabilities": {}})

    def _read(self):
        header = b""
        while not header.endswith(b":"):
            header += self.sock.recv(1)
        size = int(header[:-1])
        data = b""
        while len(data) < size:
            data += self.sock.recv(size - len(data))
        return json.loads(data)

    def cmd(self, name, params=None):
        self.msg_id += 1
        body = json.dumps([0, self.msg_id, name, params or {}]).encode()
        self.sock.sendall(str(len(body)).encode() + b":" + body)
        while True:
            reply = self._read()
            if reply[0] == 1 and reply[1] == self.msg_id:
                if reply[2]:
                    raise RuntimeError(reply[2])
                return reply[3]

    def goto(self, url):
        self.cmd("WebDriver:Navigate", {"url": url})

    def js(self, script):
        """Execute `script` dans la page ; `return ...` pour recuperer une valeur."""
        return self.cmd("WebDriver:ExecuteScript", {"script": script, "args": []})["value"]

    def js_async(self, script):
        """Script asynchrone : appeler `arguments[arguments.length - 1](valeur)` pour terminer."""
        return self.cmd("WebDriver:ExecuteAsyncScript", {"script": script, "args": []})["value"]

    def text(self, selector):
        return self.js(f"return document.querySelector({json.dumps(selector)})?.textContent ?? null")

    def click(self, selector):
        self.js(f"document.querySelector({json.dumps(selector)}).click()")

    def wait_for(self, predicate_js, timeout=10, step=0.2):
        """Attend que `predicate_js` (expression JS) soit vraie ; renvoie True/False."""
        end = time.time() + timeout
        while time.time() < end:
            if self.js(f"return Boolean({predicate_js})"):
                return True
            time.sleep(step)
        return False


class Report:
    """Compte les verifications et affiche PASS/FAIL."""

    def __init__(self):
        self.failures = 0

    def check(self, name, ok, detail=""):
        print(f"{'PASS' if ok else 'FAIL'}  {name:<52} {detail}")
        self.failures += 0 if ok else 1

    def finish(self):
        print(f"=> {self.failures} echec(s)")
        raise SystemExit(1 if self.failures else 0)
