"""Inspect both installers without displaying or exporting private credentials."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib

root = Path(__file__).resolve().parents[2]
dist = root / "dist"
version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
payload_hash = hashlib.sha256((dist / f"ChickLink-windows-x64-v{version}.exe").read_bytes()).digest()
identities = {
    role: json.loads((dist / "cloud-20260912" / role / "chicklink-connection.json").read_text())
    for role in ("mac", "windows")
}
for private in (False, True):
    installer = (dist / f"private-v{version}/ChickLink-Setup-Private-v{version}.exe" if private
                 else dist / f"ChickLink-Setup-v{version}.exe")
    with tempfile.TemporaryDirectory(prefix="chicklink-installer-check-") as folder:
        result = subprocess.run(["7zz", "x", "-y", "-o" + folder, str(installer)],
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        assert result.returncode == 0, "Installer extraction failed"
        base = Path(folder)
        assert hashlib.sha256((base / "pet-client.exe").read_bytes()).digest() == payload_hash
        configs = list(base.rglob("connection.json"))
        assert len(configs) == int(private), "Unexpected identity payload"
        if private:
            settings = json.loads(configs[0].read_text())
            assert settings == identities["windows"], "Wrong Windows identity"
            assert settings["discover_lan_relay"] is False
            assert settings["relay_url"] == "wss://pet.narendra.cn:18443/ws"
        for path in base.rglob("*"):
            if not path.is_file():
                continue
            data = path.read_bytes()
            assert identities["mac"]["access_token"].encode() not in data, "Mac identity leak"
            if not private:
                assert identities["windows"]["access_token"].encode() not in data, "Public identity leak"
    print(installer.name + ": payload and credential separation verified")
