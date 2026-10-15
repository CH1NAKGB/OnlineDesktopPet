#!/bin/sh
set -eu
SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TEST_DIR=$(mktemp -d /tmp/chick-relay-smoke.XXXXXX)
echo "Isolated test directory: $TEST_DIR"
mkdir -p "$TEST_DIR/deploy" "$TEST_DIR/bin"
cp "$SOURCE/../Cargo.toml" "$TEST_DIR/Cargo.toml"
cp "$SOURCE/relay.sh" "$SOURCE/init-encrypted-direct.sh" "$SOURCE/.env.example" \
    "$SOURCE/docker-compose.encrypted-direct.yml" "$TEST_DIR/deploy/"
cp "$SOURCE/tests/fake-docker.sh" "$TEST_DIR/bin/docker"
chmod +x "$TEST_DIR/bin/docker"
export PATH="$TEST_DIR/bin:$PATH"
sh "$TEST_DIR/deploy/relay.sh" init >/dev/null 2>&1
openssl pkey -in "$TEST_DIR/deploy/secrets/relay-private.pem" -check -noout
before=$(shasum -a 256 "$TEST_DIR/deploy/secrets/relay-private.pem" "$TEST_DIR/deploy/secrets/access-tokens.json")
sh "$TEST_DIR/deploy/relay.sh" init
after=$(shasum -a 256 "$TEST_DIR/deploy/secrets/relay-private.pem" "$TEST_DIR/deploy/secrets/access-tokens.json")
[ "$before" = "$after" ] || { echo "Credentials changed!" >&2; exit 1; }
mv "$TEST_DIR/deploy/secrets/client-tokens.env" "$TEST_DIR/saved-token-file"
if sh "$TEST_DIR/deploy/relay.sh" init >/dev/null 2>&1; then
    echo "Partial secrets were accepted!" >&2; exit 1
fi
mv "$TEST_DIR/saved-token-file" "$TEST_DIR/deploy/secrets/client-tokens.env"
for action in up status check stop; do sh "$TEST_DIR/deploy/relay.sh" "$action"; done
if CHICK_TEST_HEALTH_EXIT=22 sh "$TEST_DIR/deploy/relay.sh" check; then
    echo "Failed health check was accepted!" >&2; exit 1
fi
if sh "$TEST_DIR/deploy/relay.sh" unknown >/dev/null 2>&1; then exit 1; fi
echo "PASS: init is idempotent; partial secrets and unhealthy checks fail; Compose commands are wired correctly."
echo "Docker was stubbed; this does not validate the container runtime."
