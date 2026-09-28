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
