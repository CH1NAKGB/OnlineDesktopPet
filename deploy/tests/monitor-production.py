#!/usr/bin/env python3
"""Public, credential-free checks for the dedicated Chick Link endpoint."""

import datetime as dt
import os
import socket
import ssl
import urllib.error
import urllib.request

HOST = "pet.narendra.cn"
PORT = 18443
BASE = f"https://{HOST}:{PORT}"
MIN_TLS_DAYS = 30


def main() -> None:
    expected = os.environ.get("CHICKLINK_EXPECTED_VERSION", "")
    if not expected:
        raise SystemExit("CHICKLINK_EXPECTED_VERSION is not configured")

    context = ssl.create_default_context()
    with socket.create_connection((HOST, PORT), timeout=10) as connection:
        with context.wrap_socket(connection, server_hostname=HOST) as tls:
            expiry = dt.datetime.fromtimestamp(
                ssl.cert_time_to_seconds(tls.getpeercert()["notAfter"]), dt.timezone.utc
            )
    days_left = (expiry - dt.datetime.now(dt.timezone.utc)).total_seconds() / 86400
    print(f"TLS expires {expiry.isoformat()}, {days_left:.1f} days remaining")
    if days_left < MIN_TLS_DAYS:
        raise SystemExit(f"TLS certificate has fewer than {MIN_TLS_DAYS} days remaining")

    for path, wanted in (("/healthz", "ok"), ("/version", expected)):
        with urllib.request.urlopen(BASE + path, timeout=10, context=context) as response:
            body = response.read(128).decode("utf-8").strip()
        if body != wanted:
            raise SystemExit(f"{path} mismatch: expected {wanted!r}, got {body!r}")
        print(f"{path}: {body}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, urllib.error.URLError) as error:
        raise SystemExit(f"production monitor request failed: {error}") from None
