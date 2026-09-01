# CentOS 7.9 Deployment Notes

The relay container is x86_64-compatible and can run on the Alibaba Cloud CentOS 7.9 host or an x64 Synology Docker host. Build the image on the target host, rather than copying a macOS binary.

The distributable `ChickLink-server-docker-source.zip` contains only the Rust workspace files and deployment configuration required for this build. Extract it on the server before running the commands below; generated `target/` files are intentionally excluded.

CentOS 7 reached end-of-life on June 30, 2024, and current Docker documentation only lists maintained CentOS Stream releases. Keep this machine limited to a short-lived relay test. Before opening it to users, replace the system with Alibaba Cloud Linux 3, Rocky Linux 9, or Ubuntu 24.04 LTS. See `deployment-intake.md` for the access and network information needed for a managed deployment.

## Relay Host

1. Install a current Docker Engine and Docker Compose plugin using the vendor-supported package source available for the server at deployment time.
2. Point the intended public hostname's DNS A/AAAA record to the Alibaba Cloud server.
3. In the repository root, create `deploy/.env` from `.env.example` and set `PUBLIC_HOST`.
4. Open inbound TCP 80 and 443 in both the Alibaba Cloud security group and the OS firewall. Keep TCP 8080 private; Caddy reaches it over the Compose network.
5. Start the service with:

```bash
docker compose --env-file deploy/.env -f deploy/docker-compose.yml up -d --build
```

The Caddy container supplies HTTPS and WSS. The relay image includes its own Debian runtime, so the CentOS host's older glibc does not affect the Rust executable.

## NAS Role

Use the x64 Synology primarily for image storage, backups, and optional trusted-LAN relay testing. For public traffic, keep authentication and the WebSocket entrypoint on Alibaba Cloud. Do not expose the NAS SMB service or its DDNS address directly to desktop clients.

For a LAN-only test relay on the NAS, set `LAN_DISCOVERY=true` and expose TCP 8080 plus UDP 5353 only on the trusted local network. Do not enable that setting on the public cloud host.
