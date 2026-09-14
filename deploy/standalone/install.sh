#!/bin/sh
set -eu

# First install only: never overwrite a prior installation or existing account.
test "$(id -u)" = 0
test ! -e /opt/chicklink
test ! -e /etc/systemd/system/chicklink-relay.service
test ! -e /etc/systemd/system/chicklink-tls.service
if getent passwd chicklink >/dev/null; then
    echo 'chicklink account already exists; inspect before proceeding' >&2
    exit 1
fi
if ss -lnt | grep -Eq ':(18443|18080)[[:space:]]'; then
    echo 'A required port is already occupied' >&2
    exit 1
fi
SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
useradd --system --user-group --home-dir /opt/chicklink/state --no-create-home --shell /sbin/nologin chicklink
install -d -m 755 /opt/chicklink /opt/chicklink/bin /opt/chicklink/config
install -d -m 750 -o root -g chicklink /opt/chicklink/secrets
install -d -m 700 -o chicklink -g chicklink /opt/chicklink/state
install -m 755 "$SOURCE/relay-server" "$SOURCE/caddy" /opt/chicklink/bin/
install -m 644 "$SOURCE/Caddyfile" /opt/chicklink/config/Caddyfile
for file in relay-private.pem access-tokens.json fullchain.pem privkey.pem; do
    install -m 640 -o root -g chicklink "$SOURCE/$file" "/opt/chicklink/secrets/$file"
done
install -m 644 "$SOURCE/chicklink-relay.service" "$SOURCE/chicklink-tls.service" /etc/systemd/system/
runuser -u chicklink -- /opt/chicklink/bin/caddy validate --config /opt/chicklink/config/Caddyfile --adapter caddyfile
systemd-analyze verify /etc/systemd/system/chicklink-relay.service /etc/systemd/system/chicklink-tls.service
systemctl daemon-reload
systemctl start chicklink-relay.service chicklink-tls.service
attempt=0
until curl --fail --silent --show-error http://127.0.0.1:18080/healthz; do
    attempt=$((attempt + 1))
    test "$attempt" -lt 10
    sleep 1
done
systemctl enable chicklink-relay.service chicklink-tls.service
