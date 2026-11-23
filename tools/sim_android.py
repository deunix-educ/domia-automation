#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Telephone Android SIMULE (E2E, R5) : client WebSocket minimal (bibliotheque standard, RFC 6455) qui se comporte comme l'application
Kotlin : il ouvre `/ws/android/device/<jeton>/` sans cookie ni `Origin`, comprend `auth_ok` / `send_sms` / `pong`, envoie `sms_received`,
`call_event` et `sms_sent_ack` (memes cles JSON que `CommWebSocket.kt`), repond aux trames de controle (ping -> pong, comme OkHttp) et
peut en emettre toutes les 30 s. Aucun contact avec autre chose que l'adresse donnee.

    phone = Phone("http://127.0.0.1:8000/", token)
    phone.connect()
    phone.sms("+33699999999", "Salut", contact="Alice")
    phone.call("ringing", "0612")
    phone.wait_frame(lambda f: f.get("type") == "send_sms")
    phone.close()
"""
import base64
import json
import os
import socket
import struct
import threading
import time
import urllib.parse
from datetime import datetime


def stamp():
    """Horodatage au format de l'application (ISO_OFFSET_DATE_TIME)."""
    return datetime.now().astimezone().isoformat(timespec="seconds")


class Phone:
    def __init__(self, base_url, token, ping_interval=None, ack="ok"):
        """`ack` : `ok` (accuse de reussite), `fail` (accuse d'echec) ou `None` (aucun accuse) pour chaque `send_sms`."""
        u = urllib.parse.urlsplit(base_url)
        self.host, self.port = u.hostname, u.port or 80
        self.path = f"/ws/android/device/{token}/"
        self.ping_interval = ping_interval
        self.ack = ack
        self.frames = []  # JSON frames received from the server
        self.pings = 0  # control frames received
        self.close_code = None  # close code announced by the server
        self.connected = False
        self._sock = None
        self._lock = threading.Lock()
        self._cond = threading.Condition()

    # -- connection ---------------------------------------------------------------------------------------------------------------------------------
    def connect(self, timeout=10):
        s = socket.create_connection((self.host, self.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        s.sendall((
            f"GET {self.path} HTTP/1.1\r\nHost: {self.host}:{self.port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: okhttp/4.12.0\r\n\r\n"
        ).encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = s.recv(4096)
            if not chunk:
                raise ConnectionError("fermee pendant la poignee de main")
            head += chunk
        head, _, rest = head.partition(b"\r\n\r\n")
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise ConnectionError(head.split(b"\r\n")[0].decode())
        s.settimeout(None)
        self._sock = s
        self.connected = True
        threading.Thread(target=self._read, args=(rest,), daemon=True).start()
        if self.ping_interval:
            threading.Thread(target=self._ping_loop, daemon=True).start()

    def close(self, code=1000):
        """Fermeture propre (comme `webSocket.close(1000, ...)`)."""
        if self._sock and self.connected:
            self._write(0x8, struct.pack(">H", code))
            time.sleep(0.2)
        self._shutdown()

    def drop(self):
        """Coupure brutale (reseau perdu)."""
        self._shutdown()

    def _shutdown(self):
        self.connected = False
        if self._sock:
            try:
                self._sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self._sock.close()
            self._sock = None
        with self._cond:
            self._cond.notify_all()

    # -- what the application sends ----------------------------------------------------------------------------------------------------------
    def send(self, obj):
        self._write(0x1, json.dumps(obj).encode())

    def sms(self, sender, body, contact="", timestamp=None):
        self.send({"type": "sms_received", "from": sender, "body": body, "contact_name": contact, "timestamp": timestamp or stamp()})

    def call(self, state, number, contact="", duration=0, timestamp=None):
        self.send({"type": "call_event", "state": state, "number": number, "contact_name": contact, "timestamp": timestamp or stamp(), "duration": duration})

    def sms_ack(self, message_id, success=True, error=""):
        self.send({"type": "sms_sent_ack", "message_id": message_id, "success": success, "error": error})

    # -- what the server sends ---------------------------------------------------------------------------------------------------------------
    def wait_frame(self, predicate, timeout=10):
        """Premiere trame (deja recue ou a venir) qui satisfait `predicate`, ou None apres `timeout` secondes."""
        end = time.time() + timeout
        seen = 0
        with self._cond:
            while True:
                for f in self.frames[seen:]:
                    if predicate(f):
                        return f
                seen = len(self.frames)
                left = end - time.time()
                if left <= 0 or not self.connected and seen == len(self.frames) and self.close_code is not None:
                    return None
                self._cond.wait(min(left, 0.5))

    def wait_closed(self, timeout=10):
        end = time.time() + timeout
        with self._cond:
            while self.connected and time.time() < end:
                self._cond.wait(0.2)
        return not self.connected

    # -- frames -----------------------------------------------------------------------------------------------------------------------------------------
    def _write(self, opcode, payload):
        mask = os.urandom(4)
        n = len(payload)
        head = bytes([0x80 | opcode])
        if n < 126:
            head += bytes([0x80 | n])
        elif n < 65536:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", n)
        body = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self._lock:
            if self._sock:
                self._sock.sendall(head + mask + body)

    def _exact(self, n, buf):
        while len(buf[0]) < n:
            chunk = self._sock.recv(65536)
            if not chunk:
                raise ConnectionError("fin de flux")
            buf[0] += chunk
        out, buf[0] = buf[0][:n], buf[0][n:]
        return out

    def _read(self, rest):
        buf = [rest]
        try:
            while self.connected:
                b1, b2 = self._exact(2, buf)
                opcode, n = b1 & 0x0F, b2 & 0x7F
                if n == 126:
                    n = struct.unpack(">H", self._exact(2, buf))[0]
                elif n == 127:
                    n = struct.unpack(">Q", self._exact(8, buf))[0]
                payload = self._exact(n, buf)
                if opcode == 0x1:
                    frame = json.loads(payload.decode())
                    with self._cond:
                        self.frames.append(frame)
                        self._cond.notify_all()
                    self._on_frame(frame)
                elif opcode == 0x9:  # server ping: OkHttp answers by itself
                    self.pings += 1
                    self._write(0xA, payload)
                elif opcode == 0x8:
                    self.close_code = struct.unpack(">H", payload[:2])[0] if len(payload) >= 2 else 1005
                    self._write(0x8, payload[:2])  # OkHttp returns the same code
                    break
        except (OSError, ConnectionError, ValueError):
            pass
        finally:
            self._shutdown()

    def _on_frame(self, frame):
        if frame.get("type") == "send_sms" and self.ack:
            if self.ack == "ok":
                self.sms_ack(frame["message_id"], True)
            else:
                self.sms_ack(frame["message_id"], False, "pas de reseau")

    def _ping_loop(self):
        while self.connected:
            time.sleep(self.ping_interval)
            if self.connected:
                try:
                    self._write(0x9, b"")
                except OSError:
                    return
