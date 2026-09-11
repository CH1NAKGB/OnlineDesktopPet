"""Create private, role-specific LAN test packages; never print tokens."""
import argparse
import json
import os
from pathlib import Path
import shutil

parser = argparse.ArgumentParser()
parser.add_argument("--host", required=True)
parser.add_argument("--port", type=int, default=18081)
parser.add_argument("--secrets", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
os.umask(0o077)
tokens = dict(line.split("=", 1) for line in
              (args.secrets / "client-tokens.env").read_text().splitlines() if line)
for role, token_key, discover in [
    ("mac", "CHICK_OWNER_A_ACCESS_TOKEN", False),
    ("windows", "CHICK_OWNER_B_ACCESS_TOKEN", True),
]:
    directory = args.output / role
    directory.mkdir(parents=True, exist_ok=True)
    config = {
        "relay_url": f"ws://{args.host}:{args.port}/ws",
        "access_token": tokens[token_key],
        "discover_lan_relay": discover,
        "server_rsa_public_key_path": "relay-public.pem",
    }
    (directory / "chicklink-connection.json").write_text(json.dumps(config, indent=2) + "\n")
    shutil.copyfile(args.secrets / "relay-public.pem", directory / "relay-public.pem")
    if role == "windows":
        shutil.copyfile("target/x86_64-pc-windows-gnu/release/pet-client.exe", directory / "pet-client.exe")
        shutil.copytree("pet-client/assets/fonts", directory / "font-licenses", dirs_exist_ok=True)
        (directory / "Start-ChickLink.cmd").write_bytes(
            b'@echo off\r\ncd /d "%~dp0"\r\nset "CHICK_CONFIG_PATH=%~dp0chicklink-connection.json"\r\n'
            b'set "CHICK_RELAY_URL="\r\nset "CHICK_ACCESS_TOKEN="\r\n'
            b'set "CHICK_SERVER_RSA_PUBLIC_KEY_PATH="\r\nset "CHICK_TLS_CA_CERT_PATH="\r\n'
            b'set "CHICK_DISCOVER_LAN="\r\nset "CHICK_OPEN_CHAT=1"\r\n'
            b'pet-client.exe\r\nif errorlevel 1 pause\r\n')
print(f"LAN packages prepared for {args.host}:{args.port}; credentials not printed.")
