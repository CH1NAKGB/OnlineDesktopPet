"""Verify public v0.3.4 archives do not contain private connection artifacts."""

import json
from pathlib import Path

from public_archive import check_archive


ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"
VERSION = "0.3.4"


def known_private_tokens() -> tuple[bytes, ...]:
    tokens = []
    for role in ("mac", "windows"):
        path = DIST / "cloud-20260912" / role / "chicklink-connection.json"
        if path.is_file():
            tokens.append(json.loads(path.read_text())["access_token"].encode())
    for directory in (ROOT / "deploy/lan-test.local", ROOT / "deploy/secrets/aliyun"):
        env = directory / "client-tokens.env"
        if env.is_file():
            for line in env.read_bytes().splitlines():
                if b"=" in line and not line.startswith(b"#"):
                    tokens.append(line.partition(b"=")[2].strip())
        mapping = directory / "access-tokens.json"
        if mapping.is_file():
            tokens.extend(key.encode() for key in json.loads(mapping.read_text()))
    return tuple(token for token in tokens if len(token) >= 16)


def main() -> None:
    tokens = known_private_tokens()
    archives = (
        DIST / f"ChickLink-server-docker-source-v{VERSION}.zip",
        DIST / f"ChickLink-windows-x64-v{VERSION}.zip",
    )
    for archive in archives:
        check_archive(archive, tokens)
        print(f"{archive.name}: public archive check passed")
    binary = DIST / f"ChickLink-windows-x64-v{VERSION}.exe"
    data = binary.read_bytes()
    if any(token in data for token in tokens):
        raise ValueError("Known private token in public executable")
    print("Public executable token check passed")


if __name__ == "__main__":
    main()
