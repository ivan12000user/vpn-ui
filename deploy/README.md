# vpn-ui deployment files

This directory contains only the current production artifacts and reusable maintenance tools.
One-off migration/repair and completed legacy-UI cleanup scripts are kept in Git history rather than in the active deployment
directory.

## Production artifact mapping

### Management helper

- `libexec/vpn-ui-manage`
  - installed as `/usr/local/libexec/vpn-ui-manage`
  - current wrapper and safe-settings layer
  - delegates peer CRUD to the versioned core
  - owns `settings_get` and `settings_edit`

- `libexec/vpn-ui-manage-core-v070`
  - installed as `/usr/local/libexec/vpn-ui-manage-core-v070`
  - v0.7.0 peer-management core
  - supports defaults, list, get, create, edit, enable, disable and delete
  - performs backups, inventory validation and live-state verification
  - applies peer changes with `syncconf`, without restarting the VPN interfaces

- `bin/vpn-ui-manage-wg`
  - installed as `/usr/local/bin/vpn-ui-manage-wg`

- `bin/vpn-ui-manage-awg`
  - installed as `/usr/local/bin/vpn-ui-manage-awg`

### Web access

- `nginx/vpn-ui-global.conf`
  - current Nginx site configuration
  - HTTP redirects to HTTPS
  - the whole UI is protected at the HTTPS entry point
  - the application itself remains bound to `127.0.0.1:8090`

- `install-local-ca.sh`
  - reusable private-CA certificate installation/rotation helper
  - preserves VPN interface state

### systemd

- `systemd/vpn-ui.service.d/manage-write.conf`
  - keeps `ProtectSystem=strict`
  - grants write access only to the WireGuard and AmneziaWG configuration directories

## Network design

- WireGuard `wg0`: `10.66.66.0/24`, IPv4-only inside the tunnel.
- AmneziaWG `awg0`: `10.77.77.0/24`, IPv4-only inside the tunnel.
- Public IPv6 on the VPS remains enabled and is independent of the internal WG addressing.
- `fd66:66:66::/64` was an experiment and must not appear in WG interface addresses,
  peer AllowedIPs, vpn-ui inventory or NAT rules.

## Verification

Run from the repository checkout on the administrative host:

```bash
bash deploy/verify-production.sh
```

The verifier is read-only and checks:

- `wg-quick@wg0`, `awg-quick@awg0`, `vpn-ui` and Nginx are active;
- Nginx configuration is valid;
- retired UI systemd units, TCP listeners, files and the `awg-web` account/group are absent;
- inventory, persistent configuration and live peer counts agree;
- WireGuard inventory/config/live state is IPv4-only;
- `wg0` has `10.66.66.1/24` and no IPv6 address;
- no `fd66:66:66::/64` NAT66 rule remains;
- the VPS still has global IPv6 and an IPv6 default route;
- saved defaults are `10.66.66.0/24` for WG and `10.77.77.0/24` for AWG;
- the local UI health endpoint and Nginx authentication entry point respond correctly.

The runtime inventory and backups remain under `/var/lib/vpn-ui`.
