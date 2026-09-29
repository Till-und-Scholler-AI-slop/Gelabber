"""Owned loopback WebSocket with a genuinely stalled TCP receive window.

Credentials arrive on stdin, never argv/logs. Only scalar outcomes go to stdout.
"""
import base64
import hashlib
import json
import os
import select
import socket
import struct
import sys
import time


def output(value):
    print(json.dumps(value), flush=True)


sock = None
try:
    fixture = json.loads(sys.stdin.readline())
    assert fixture["host"] == "127.0.0.1"
    sock = socket.socket()
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
    sock.settimeout(5)
    sock.connect((fixture["host"], fixture["port"]))
    key = base64.b64encode(os.urandom(16)).decode()
    assert "\r" not in fixture["cookie"] and "\n" not in fixture["cookie"]
    host = f'127.0.0.1:{fixture["port"]}'
    sock.sendall((f"GET /ws HTTP/1.1\r\nHost: {host}\r\n"
                  f"Origin: http://{host}\r\nUpgrade: websocket\r\n"
                  f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                  f'Sec-WebSocket-Version: 13\r\nCookie: {fixture["cookie"]}\r\n\r\n').encode())
    header = b""
    while not header.endswith(b"\r\n\r\n"):
        data = sock.recv(1)
        assert data and len(header) < 16384
        header += data
    assert header.split(b"\r\n", 1)[0].split()[1] == b"101"
    expected = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
    assert expected.lower() in header.lower()

    def send(payload, opcode=1):
        payload = payload.encode() if isinstance(payload, str) else payload
        mask = os.urandom(4)
        assert len(payload) < 126
        sock.sendall(bytes([0x80 | opcode, 0x80 | len(payload)]) + mask +
                     bytes(v ^ mask[i % 4] for i, v in enumerate(payload)))

    def exact(size):
        data = b""
        while len(data) < size:
            chunk = sock.recv(size - len(data))
            assert chunk
            data += chunk
        return data

    def frame():
        a, b = exact(2)
        assert not b & 0x80
        length = b & 127
        if length == 126:
            length = struct.unpack("!H", exact(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", exact(8))[0]
        assert length < 100000
        payload = exact(length)
        if a & 15 == 9:
            send(payload, 10)
            return {}
        assert a & 15 == 1
        parsed = json.loads(payload)
        if parsed.get("op") == "h":
            send('{"op":"h"}')
        return parsed

    send(json.dumps({"op": "s", "s": fixture["server"], "c": fixture["channel"]}, separators=(",", ":")))
    while True:
        item = frame()
        if item.get("op") == "ok" and item.get("c") == fixture["channel"]:
            break
    output({"subscribed": True, "port": sock.getsockname()[1],
            "receiveBufferBytes": sock.getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF)})
    while True:
        command = json.loads(sys.stdin.readline())
        if command["op"] == "control":
            while True:
                if frame().get("op") == "e":
                    break
            output({"positiveEvent": True})
        elif command["op"] == "pause":
            output({"paused": True})
            break
    # No recv calls beyond this point. Client heartbeats keep the test independent
    # of the 30s silent-client timeout, while the peer's TCP window stays stalled.
    while True:
        if select.select([sys.stdin], [], [], 0.5)[0]:
            line = sys.stdin.readline()
            if not line or json.loads(line)["op"] == "close":
                break
        try:
            send('{"op":"h"}')
        except OSError:
            break
except Exception:
    output({"error": "owned-raw-reader-interface"})
finally:
    if sock is not None:
        sock.close()
