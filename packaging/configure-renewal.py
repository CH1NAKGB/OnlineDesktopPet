"""Collect renewal-only credentials locally without echoing or overwriting them."""
import getpass
import json
import os
from pathlib import Path
import sys


def main():
    if not sys.stdin.isatty():
        raise SystemExit("Please run this tool yourself in the Mac Terminal.")
    directory = Path(__file__).resolve().parents[1] / "deploy" / "secrets" / "renewal"
    target = directory / "credentials.json"
    if target.exists():
        raise SystemExit("Renewal credentials already exist; nothing was overwritten.")
    print("Enter the AccessKey belonging ONLY to chicklink-cert.")
    print("Both fields are hidden while typing/pasting. Do not send them in chat.")
    access_key = getpass.getpass("AccessKey ID (hidden): ").strip()
    secret = getpass.getpass("AccessKey Secret (hidden): ").strip()
    if not access_key or not secret or any(c.isspace() for c in access_key + secret):
        raise SystemExit("Missing value or unexpected whitespace; nothing was saved.")
    os.umask(0o077)
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump({"access_key_id": access_key, "access_key_secret": secret}, output)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        target.unlink(missing_ok=True)
        raise
    print("Renewal credentials saved privately. Workbench configuration unchanged.")


if __name__ == "__main__":
    try:
        main()
    except (KeyboardInterrupt, EOFError):
        raise SystemExit("\nCancelled.") from None
