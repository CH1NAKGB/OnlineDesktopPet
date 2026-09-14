#!/bin/bash
set -euo pipefail
umask 077
mode=${1:-production}
case "$mode" in staging|production) ;; *) exit 2 ;; esac
base=/opt/chicklink/renewal
exec 9>"$base/renew.lock"
flock -n 9 || exit 0
export ALICLOUD_ACCESS_KEY_FILE="$base/access-key-id"
export ALICLOUD_SECRET_KEY_FILE="$base/access-key-secret"
export ALICLOUD_REGION_ID=cn-hangzhou
export ALICLOUD_TTL=600
export ALICLOUD_PROPAGATION_TIMEOUT=900
export ALICLOUD_POLLING_INTERVAL=5
export ALICLOUD_HTTP_TIMEOUT=20
export GOMEMLIMIT=160MiB GOMAXPROCS=1
server=letsencrypt
directory=https://acme-v02.api.letsencrypt.org/directory
if [ "$mode" = staging ]; then
  server=letsencrypt-staging
  directory=https://acme-staging-v02.api.letsencrypt.org/directory
fi
# Fail before DNS changes when the CA cannot be reached from this host.
if ! curl --fail --silent --show-error --connect-timeout 8 --max-time 20 \
  --output /dev/null "$directory"; then
  echo 'ACME endpoint unreachable; current certificate and TLS service unchanged.' >&2
  exit 1
fi
/opt/chicklink/bin/lego run --accept-tos --account-id chicklink-cert \
  --domains pet.narendra.cn --server "$server" --path "$base/$mode" \
  --dns alidns --http-timeout 30 --renew-days 30 --no-random-sleep
if [ "$mode" = production ]; then
  /opt/chicklink/bin/deploy-certificate.sh
fi
