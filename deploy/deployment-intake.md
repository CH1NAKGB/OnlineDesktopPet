# Server Deployment Intake

## Recommended topology

Current owner constraints: CentOS 7.9 x64, host ports 80/443/8080 occupied, proposed public port 18080. Use the encrypted-direct configuration and the tested v0.3.3 Linux x64 image for the initial deployment. No actual ECS deployment has been completed yet. See [QUICKSTART.md](QUICKSTART.md) and [offread.md](../offread.md).

- Alibaba Cloud ECS: public WSS entrypoint, Caddy, relay server, authentication API later.
- NAS: backups and future image/media storage. Do not expose SMB to the public internet.
- Preferred host OS: Alibaba Cloud Linux 3, Rocky Linux 9, or Ubuntu 24.04 LTS.

CentOS 7 can be used for a short-lived test, but it no longer receives security updates and is
not listed by Docker as a currently supported CentOS release. Migrate the ECS system disk before
long-term public deployment.

## Information needed from the owner

1. ECS public IPv4 address or EIP.
2. SSH port and login username.
3. Confirmation that the login can run `sudo` without sharing the sudo password in chat.
4. Authentication method: an SSH public key installed on the server is preferred.
5. Current output of `cat /etc/os-release`, `uname -m`, `df -h /`, and `free -h`.
6. Whether Docker Engine and the Compose plugin are already installed.
7. Domain/subdomain to use, if any, and confirmation that its A record can point to the ECS IP.
8. Current Alibaba Cloud security-group inbound rules for TCP 22, 80, 443, 8080, and the proposed alternate TCP 18080.
9. Desired deployment mode:
   - Domain + Caddy + `wss://` (recommended for public use).
   - Direct IP + RSA/AES application encryption on TCP 18080 (test/private deployment).
10. Whether the server may be rebooted or its operating system may be replaced.

## Do not send in chat

- Root or account password.
- RSA private key contents.
- Alibaba Cloud AccessKey secret.
- NAS administrator password.

Install an SSH public key on the host instead. Keep secrets in root-readable files or Docker
secrets on the server. If temporary credentials are unavoidable, rotate them immediately after
deployment.

## Network rules

For the recommended WSS deployment, expose TCP 80 and 443 publicly. Restrict SSH to the owner's
current public IP. Do not expose relay port 8080 directly; Caddy reaches it on the internal Docker
network.

This is a future alternative for this owner. The existing Caddy Compose template lacks provisioned-token configuration and must be extended before public use; do not stop the owner's existing services to free those ports.

For direct-IP RSA testing, expose TCP 18080 only to the two testers' public IPs where possible.
Distribute `relay-public.pem` through the application package or another trusted channel; never
download it from the same unauthenticated relay endpoint.

## Acceptance checks

- `GET /healthz` returns `ok` through the intended public endpoint.
- A client completes the encrypted handshake and receives `Ready`.
- Invalid and `dev:<name>` tokens are rejected when provisioned-token mode is enabled.
- Two clients can register unique pairing codes and pair.
- Text messages, nickname updates, presence, and shared-pet actions work in both directions.
- Restarting the container does not lose Caddy certificates or server keys.
- Relay port 8080 is not publicly reachable in the WSS deployment.
