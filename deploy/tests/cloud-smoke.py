"""Read-only encrypted login/ping check. Does not change pairing or send chat.

Dependencies: cryptography, websockets. Credentials are read from a private file.
"""
import argparse
import base64
import json
import os
import ssl
import time
from pathlib import Path

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from websockets.sync.client import connect
from websockets.exceptions import ConnectionClosed

parser = argparse.ArgumentParser()
parser.add_argument("--config", required=True, type=Path)
args = parser.parse_args()
config = json.loads(args.config.read_text())
if not config["relay_url"].startswith("wss://"):
    parser.error("TLS is required")
public_key = serialization.load_pem_public_key(
    (args.config.parent / config["server_rsa_public_key_path"]).read_bytes()
)
b64 = lambda data: base64.b64encode(data).decode("ascii")

def check(token, expected):
    key = os.urandom(32)
    cipher = AESGCM(key)
    with connect(config["relay_url"], ssl=ssl.create_default_context(),
                 proxy=None, open_timeout=10, close_timeout=3) as socket:
        encrypted_key = public_key.encrypt(key, padding.OAEP(
            mgf=padding.MGF1(hashes.SHA256()), algorithm=hashes.SHA256(), label=None))
        socket.send(json.dumps({"type": "secure_handshake", "encrypted_session_key": b64(encrypted_key)}))

        def send(event):
            nonce = os.urandom(12)
            socket.send(json.dumps({"type": "encrypted", "frame": {
                "nonce": b64(nonce),
                "ciphertext": b64(cipher.encrypt(nonce, json.dumps(event).encode(), None))}}))

        def receive():
            frame = json.loads(socket.recv(timeout=10))["frame"]
            return json.loads(cipher.decrypt(base64.b64decode(frame["nonce"]),
                                             base64.b64decode(frame["ciphertext"]), None))

        send({"type": "hello", "protocol_version": 2, "access_token": token,
              "device_id": "deployment-smoke"})
        try:
            event = receive()
        except ConnectionClosed:
            if expected != "error":
                raise
            print("Invalid-token connection closed by relay")
            return
        assert event["type"] == expected, "unexpected authentication result"
        if expected == "error":
            assert event["code"] == "authentication_failed"
            print("Invalid token rejected over TLS + RSA/AES")
            return
        started = time.monotonic()
        send({"type": "ping"})
        for _ in range(10):
            if receive()["type"] == "pong":
                print(f"TLS + RSA/AES login and ping OK ({(time.monotonic() - started) * 1000:.0f} ms)")
                return
        raise AssertionError("missing pong")

check("invalid-deployment-test-token", "error")
check(config["access_token"], "ready")
