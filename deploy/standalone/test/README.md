# Temporary preview relay

This preview uses its own relay, SQLite database, RSA key, storage key, TLS
certificate, state directory, systemd units, and listener ports. It does not
replace or reload `chicklink-relay` or `chicklink-tls`.

- Relay: `127.0.0.1:18081`
- TLS preview: `pet.narendra.cn:39991`
- Public entry path: `wss://pet.narendra.cn:39991/ws`
- Health check: `https://pet.narendra.cn:39991/healthz`
- State and secrets: `/opt/chicklink-test/`
- The test TLS certificate is self-signed; configure the preview client to
  trust the matching `tls-cert.pem` explicitly.
- The port is chosen from the host's existing `39000-40000/tcp` firewall
  allowance. Do not add or broaden firewall rules for this preview.
- Units are started manually and are not enabled at boot.

The preview account database is disposable and is not backed up. Do not use
production credentials, recovery codes, or chat content. This service is
temporary; stop only `chicklink-tls-test` and `chicklink-relay-test` after the
test, then archive or remove `/opt/chicklink-test` in a separate cleanup step.
