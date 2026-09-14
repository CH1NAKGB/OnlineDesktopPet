#!/bin/bash
set -euo pipefail
umask 077
source_dir=/opt/chicklink/renewal/production/certificates
live=/opt/chicklink/secrets
cert="$source_dir/pet.narendra.cn.crt"
key="$source_dir/pet.narendra.cn.key"
test -s "$cert" && test -s "$key"
openssl x509 -in "$cert" -noout -checkend 2592000
openssl x509 -in "$cert" -noout -text | grep -q 'DNS:pet.narendra.cn'
cert_pub=$(openssl x509 -in "$cert" -pubkey -noout | openssl pkey -pubin -outform DER | sha256sum)
key_pub=$(openssl pkey -in "$key" -pubout -outform DER | sha256sum)
test "$cert_pub" = "$key_pub"
openssl verify -purpose sslserver -CAfile /etc/pki/tls/certs/ca-bundle.crt -untrusted "$cert" "$cert"
if cmp -s "$cert" "$live/fullchain.pem" && cmp -s "$key" "$live/privkey.pem"; then
  echo 'Certificate unchanged; TLS service not restarted.'
  exit 0
fi
backup=$(mktemp -d /opt/chicklink/renewal/rollback.XXXXXXXX)
cp -p "$live/fullchain.pem" "$live/privkey.pem" "$backup/"
rollback() {
  trap - ERR
  install -o root -g chicklink -m 640 "$backup/fullchain.pem" "$live/fullchain.pem"
  install -o root -g chicklink -m 640 "$backup/privkey.pem" "$live/privkey.pem"
  systemctl restart chicklink-tls
  echo 'Certificate deployment failed; previous certificate restored.' >&2
  exit 1
}
trap rollback ERR
install -o root -g chicklink -m 640 "$cert" "$live/fullchain.pem.new"
install -o root -g chicklink -m 640 "$key" "$live/privkey.pem.new"
mv "$live/fullchain.pem.new" "$live/fullchain.pem"
mv "$live/privkey.pem.new" "$live/privkey.pem"
runuser -u chicklink -- /opt/chicklink/bin/caddy validate --config /opt/chicklink/config/Caddyfile --adapter caddyfile
systemctl restart chicklink-tls
healthy=false
for attempt in 1 2 3 4 5; do
  if curl --noproxy '*' --fail --silent --show-error \
    --connect-timeout 5 --max-time 15 --resolve pet.narendra.cn:18443:127.0.0.1 \
    https://pet.narendra.cn:18443/healthz; then
    healthy=true
    break
  fi
  sleep 2
done
test "$healthy" = true
trap - ERR
echo 'Production certificate deployed and HTTPS verified.'
