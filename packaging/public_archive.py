"""Reject private material before a public archive is published."""

from pathlib import Path, PurePosixPath
import re
import zipfile


PRIVATE_KEY = re.compile(
    rb"-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"
)
FORBIDDEN_DIRS = {"secrets", "lan-test.local"}
FORBIDDEN_NAMES = {
    ".env",
    "acme.json",
    "access-tokens.json",
    "client-tokens.env",
    "chicklink-connection.json",
    "connection.json",
    "relay-private.pem",
    "id_rsa",
}
FORBIDDEN_SUFFIXES = (".key", ".p12", ".pfx")


def check_member(name: str, data: bytes) -> None:
    path = PurePosixPath(name)
    parts = path.parts
    if not parts or name.startswith("/") or ".." in parts or "\\" in name:
        raise ValueError(f"Unsafe archive path: {name}")
    if (FORBIDDEN_DIRS.intersection(parts)
            or path.name in FORBIDDEN_NAMES
            or (path.name.startswith(".env.")
                and path.name not in {".env.example", ".env.sample"})
            or path.name.endswith(FORBIDDEN_SUFFIXES)):
        raise ValueError(f"Private path in public archive: {name}")
    if PRIVATE_KEY.search(data):
        raise ValueError(f"Private key in public archive: {name}")


def check_archive(path: Path, tokens: tuple[bytes, ...] = ()) -> None:
    with zipfile.ZipFile(path) as archive:
        bad_member = archive.testzip()
        if bad_member is not None:
            raise ValueError(f"Corrupt archive member: {bad_member}")
        for info in archive.infolist():
            data = archive.read(info)
            check_member(info.filename, data)
            if any(token in data for token in tokens):
                raise ValueError(f"Known private token in public archive: {info.filename}")
