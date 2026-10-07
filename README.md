# VPN UI

Unified self-hosted Web UI for WireGuard and AmneziaWG.

Current application version: **0.7.0**.

## Production model

The application runs on the VPN server and is exposed through Nginx. Development machines,
VPN peers and GitHub are not runtime dependencies.

Current internal VPN addressing:

- WireGuard `wg0`: `10.66.66.0/24` — IPv4-only inside the tunnel.
- AmneziaWG `awg0`: `10.77.77.0/24` — IPv4-only inside the tunnel.
- Public IPv6 on the VPS is intentionally preserved for the host and HTTPS/Nginx.
- The former experimental WireGuard ULA `fd66:66:66::/64` is not part of the supported design.

The UI process listens on `127.0.0.1:8090`. Nginx provides the external HTTPS entry point
and protects the whole UI with HTTP Basic authentication.

## Management model

Peer management uses a privileged wrapper plus a versioned management core:

- `/usr/local/libexec/vpn-ui-manage` — current wrapper and safe-settings layer.
- `/usr/local/libexec/vpn-ui-manage-core-v070` — v0.7.0 peer-management core.
- `/usr/local/bin/vpn-ui-manage-wg` — WireGuard sudo wrapper.
- `/usr/local/bin/vpn-ui-manage-awg` — AmneziaWG sudo wrapper.

Peer create/edit/enable/disable/delete operations use backups, inventory checks and
`syncconf`; they do not require restarting `wg0` or `awg0`.

## Repository layout

- `src/` — Rust/Axum application.
- `deploy/libexec/` — privileged management wrapper and versioned core.
- `deploy/bin/` — provider-specific command wrappers.
- `deploy/nginx/` — current Nginx configuration.
- `deploy/systemd/` — systemd drop-ins required by the UI.
- `deploy/verify-production.sh` — read-only production consistency audit.
- `deploy/install-local-ca.sh` — private-CA certificate installation/rotation helper.

Historical one-off deployment, repair, IPv6-cleanup and completed legacy-UI cleanup scripts are intentionally not kept in
the active deployment tree. They remain available in Git history if an old incident needs to
be reconstructed.

## Safe production check

From the administrative Orange Pi repository checkout:

```bash
bash deploy/verify-production.sh
```

The verifier is read-only. It checks service health, inventory/config/live peer consistency,
the IPv4-only WireGuard design, public VPS IPv6 preservation, UI health and Nginx.
It also checks that the retired UI services, listeners, files and `awg-web` account/group are absent.
