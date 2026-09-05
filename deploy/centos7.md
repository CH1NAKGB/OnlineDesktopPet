# CentOS 7.9 Deployment Notes

The relay image targets Linux x86_64. v0.3.3 was built and health-checked on a GitHub Ubuntu runner; the owner's CentOS 7.9 and Synology hosts still need compatibility testing. Prefer the prebuilt image on low-memory machines; do not copy a macOS executable onto Linux.

Start with [QUICKSTART.md](QUICKSTART.md): import the release image and run `sh deploy/relay.sh start`, or use `sh deploy/relay.sh up` for a source build. Overall project status is in [offread.md](../offread.md).

The distributable `ChickLink-server-docker-source.zip` contains only the Rust workspace files and deployment configuration required for this build. Extract it on the server before running the commands below; generated `target/` files are intentionally excluded.

CentOS 7 reached end-of-life on June 30, 2024, and current Docker documentation only lists maintained CentOS Stream releases. Keep this machine limited to a short-lived relay test. Before opening it to users, replace the system with Alibaba Cloud Linux 3, Rocky Linux 9, or Ubuntu 24.04 LTS. See `deployment-intake.md` for the access and network information needed for a managed deployment.

## Encrypted Direct Relay On Port 18080

Use this mode when host ports 80, 443, and 8080 are already occupied. The public
socket is ordinary `ws://` on TCP 18080, but every WebSocket business event is
protected by a pinned RSA-3072 key exchange and AES-256-GCM. Random provisioned
access tokens replace the insecure `dev:<name>` identity fallback.

Run these commands from the extracted repository as the account that operates
Docker:

```bash
cp deploy/.env.example deploy/.env
./deploy/init-encrypted-direct.sh
docker compose --env-file deploy/.env \
  -f deploy/docker-compose.encrypted-direct.yml up -d --build
curl http://127.0.0.1:18080/healthz
```

The initialization script refuses to overwrite existing keys. Keep
`deploy/secrets/relay-private.pem` and `access-tokens.json` only on the server.
Give each owner only their own token from `client-tokens.env`, plus a trusted
copy of `relay-public.pem`.

Open TCP 18080 in the Alibaba Cloud security group and the host firewall. Limit
the source CIDRs to the two testers where possible. A CentOS firewalld example is:

```bash
sudo firewall-cmd --permanent --add-port=18080/tcp
sudo firewall-cmd --reload
```

Start a client with the public server address, the assigned opaque token, and
the pinned public key:

```bash
CHICK_RELAY_URL=ws://SERVER_IP:18080/ws \
CHICK_ACCESS_TOKEN=PASTE_ONLY_THIS_OWNERS_TOKEN \
CHICK_SERVER_RSA_PUBLIC_KEY_PATH=/trusted/path/relay-public.pem \
cargo run -p pet-client
```

The direct deployment sets both `RELAY_REQUIRE_ENCRYPTION=true` and
`RELAY_REQUIRE_PROVISIONED_TOKENS=true`. Missing key files therefore stop the
container instead of silently falling back to plaintext or development login.

## Domain Relay Host (Reference Only)

The older Caddy Compose file does not configure the provisioned-token map. Add real authentication before exposing that variant publicly; TLS alone does not replace authentication. The current user's host ports 80/443/8080 are occupied, so prefer the encrypted direct configuration above.

1. Install a current Docker Engine and Docker Compose plugin using the vendor-supported package source available for the server at deployment time.
2. Point the intended public hostname's DNS A/AAAA record to the Alibaba Cloud server.
3. In the repository root, create `deploy/.env` from `.env.example` and set `PUBLIC_HOST`.
4. Open inbound TCP 80 and 443 in both the Alibaba Cloud security group and the OS firewall. Keep TCP 8080 private; Caddy reaches it over the Compose network. Skip this mode when those host ports are occupied.
5. Start the service with:

```bash
docker compose --env-file deploy/.env -f deploy/docker-compose.yml up -d --build
```

The Caddy container supplies HTTPS and WSS. The relay image includes its own Debian runtime, so the CentOS host's older glibc does not affect the Rust executable.

## NAS Role

Use the x64 Synology primarily for image storage, backups, and optional trusted-LAN relay testing. For public traffic, keep authentication and the WebSocket entrypoint on Alibaba Cloud. Do not expose the NAS SMB service or its DDNS address directly to desktop clients.

For a LAN-only test relay on the NAS, set `LAN_DISCOVERY=true` and expose TCP 8080 plus UDP 5353 only on the trusted local network. Do not enable that setting on the public cloud host.
