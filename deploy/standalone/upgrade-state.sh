#!/bin/sh
set -eu
test "$(id -u)" = 0
SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
test -f "$SOURCE/relay-server"
test -f "$SOURCE/chicklink-relay.service"
runuser -u chicklink -- test -w /opt/chicklink/state
BACKUP=/opt/chicklink/backups/state-upgrade-$(date +%Y%m%d%H%M%S)
install -d -m 700 "$BACKUP"
cp /opt/chicklink/bin/relay-server "$BACKUP/relay-server"
cp /etc/systemd/system/chicklink-relay.service "$BACKUP/chicklink-relay.service"
rollback() {
    echo 'New relay health check failed; restoring previous relay' >&2
    systemctl stop chicklink-tls chicklink-relay || true
    install -m 755 "$BACKUP/relay-server" /opt/chicklink/bin/relay-server
    install -m 644 "$BACKUP/chicklink-relay.service" /etc/systemd/system/chicklink-relay.service
    systemctl daemon-reload
    systemctl start chicklink-relay chicklink-tls
}
trap rollback EXIT
systemctl stop chicklink-tls chicklink-relay
install -m 755 "$SOURCE/relay-server" /opt/chicklink/bin/relay-server
install -m 644 "$SOURCE/chicklink-relay.service" /etc/systemd/system/chicklink-relay.service
systemctl daemon-reload
systemctl start chicklink-relay chicklink-tls
attempt=0
until curl --fail --silent --show-error --resolve pet.narendra.cn:18443:127.0.0.1 https://pet.narendra.cn:18443/healthz; do
    attempt=$((attempt + 1))
    test "$attempt" -lt 10
    sleep 1
done
trap - EXIT
echo "Pairing persistence enabled; rollback files: $BACKUP"
