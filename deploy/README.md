# vpn-ui deployment files

This directory contains only deployment components that are safe to publish.

Environment-specific operational scripts, production IP addresses, hostnames, certificates and access details are intentionally not stored in the public repository.

## Included

- `libexec/vpn-ui-manage`
  - privileged WireGuard / AmneziaWG peer management helper;
  - supports defaults, list, get, create, edit, enable, disable and delete;
  - uses backups, inventory validation and live configuration verification;
  - applies peer changes without restarting `wg0` or `awg0`.

- `libexec/vpn-ui-manage-settings-wrapper`
  - safe settings wrapper for defaults used by new clients.

- `bin/vpn-ui-manage-wg`
  - sudo wrapper for WireGuard management.

- `bin/vpn-ui-manage-awg`
  - sudo wrapper for AmneziaWG management.

- `systemd/vpn-ui.service.d/manage-write.conf`
  - keeps `ProtectSystem=strict`;
  - grants write access only to the required VPN configuration directories.

## Local deployment data

Keep the following outside Git:

- real public/private VPN addresses used by your installation;
- server hostnames and SSH aliases;
- TLS private keys and local CA private keys;
- htpasswd/authentication files;
- client configs and WireGuard/AmneziaWG private keys;
- production backup files.

For deployment automation, inject installation-specific addresses through local environment variables or untracked configuration files rather than hard-coding them in repository scripts.

Runtime inventory and backups remain under `/var/lib/vpn-ui` and must not be committed.
