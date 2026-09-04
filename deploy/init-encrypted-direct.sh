#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SECRETS_DIR=${1:-"$SCRIPT_DIR/secrets"}

for path in relay-private.pem relay-public.pem access-tokens.json client-tokens.env; do
    if [ -e "$SECRETS_DIR/$path" ]; then
        echo "Refusing to overwrite existing secret: $SECRETS_DIR/$path" >&2
        exit 1
    fi
done

umask 077
mkdir -p "$SECRETS_DIR"
chmod 700 "$SECRETS_DIR"

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 \
    -out "$SECRETS_DIR/relay-private.pem"
openssl pkey -in "$SECRETS_DIR/relay-private.pem" -pubout \
    -out "$SECRETS_DIR/relay-public.pem"

OWNER_A_TOKEN=$(openssl rand -hex 32)
OWNER_B_TOKEN=$(openssl rand -hex 32)
printf '{\n  "%s": "owner-a",\n  "%s": "owner-b"\n}\n' \
    "$OWNER_A_TOKEN" "$OWNER_B_TOKEN" > "$SECRETS_DIR/access-tokens.json"
printf 'CHICK_OWNER_A_ACCESS_TOKEN=%s\nCHICK_OWNER_B_ACCESS_TOKEN=%s\n' \
    "$OWNER_A_TOKEN" "$OWNER_B_TOKEN" > "$SECRETS_DIR/client-tokens.env"

# The parent directory remains mode 0700. Read-only 0644 files let the
# non-root container user consume bind-mounted secrets without host root access.
chmod 644 "$SECRETS_DIR/relay-private.pem" "$SECRETS_DIR/access-tokens.json"
chmod 644 "$SECRETS_DIR/relay-public.pem"
chmod 600 "$SECRETS_DIR/client-tokens.env"

echo "Encrypted relay secrets created in $SECRETS_DIR"
echo "Keep relay-private.pem and access-tokens.json on the server only."
echo "Distribute relay-public.pem and each owner's own token through a trusted channel."
