# vpn-ui deployment files

This directory contains the privileged management components used by vpn-ui.

## Files

- `libexec/vpn-ui-manage`
  - privileged WireGuard / AmneziaWG peer management helper
  - supports defaults, list, get, create, edit, enable, disable and delete
  - uses backups, inventory validation and live configuration verification
  - applies changes without restarting wg0 or awg0

- `bin/vpn-ui-manage-wg`
  - sudo wrapper for WireGuard management

- `bin/vpn-ui-manage-awg`
  - sudo wrapper for AmneziaWG management

- `systemd/vpn-ui.service.d/manage-write.conf`
  - keeps `ProtectSystem=strict`
  - grants write access only to the WireGuard and AmneziaWG configuration directories

The runtime inventory and backups remain under `/var/lib/vpn-ui`.

Production paths:

- `/usr/local/libexec/vpn-ui-manage`
- `/usr/local/bin/vpn-ui-manage-wg`
- `/usr/local/bin/vpn-ui-manage-awg`
- `/etc/systemd/system/vpn-ui.service.d/manage-write.conf`

## Current network design

- WireGuard `wg0` uses IPv4-only internal addressing: `10.66.66.0/24`.
- AmneziaWG `awg0` uses IPv4-only internal addressing: `10.77.77.0/24`.
- Public IPv6 on the VPS is intentionally preserved for the host and HTTPS/Nginx.
- The experimental WireGuard ULA `fd66:66:66::/64` is not part of the supported vpn-ui design.

Maintenance helpers:

- `audit-wg-ipv6.sh`
  - read-only audit used to detect accidental or experimental WireGuard IPv6 state.
- `cleanup-wg-ipv6.sh`
  - removes only the experimental live `fd66:66:66::/64` WireGuard address/NAT66 state.
  - preserves public VPS IPv6, Tailscale IPv6, WireGuard IPv4, AmneziaWG, Nginx and vpn-ui.
  - does not restart `wg0` or `awg0`.
