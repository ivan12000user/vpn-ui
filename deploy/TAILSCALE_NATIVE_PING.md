# VPN UI v0.8.3 — Tailscale ping + ICMP

The Tailscale page has **Tailscale ping** as the primary check, showing
RTT and the latest measured path (DIRECT, DERP region, or RELAY).
Classic OS ICMP remains a separately labelled secondary check. "Проверить
все TS" runs only Tailscale-native checks, at most 4 at once.

## Security

- `POST /api/tailscale/overlay-ping` accepts an IPv4 address but only if it
  belongs to the current local tailscaled Peer map and 100.64.0.0/10.
- A root-owned, root-executed, no-argument Python helper *rechecks* the same
  address against a fresh root-readable tailscaled status. It does not accept
  command line arguments. It only runs `tailscale status --json` and
  `tailscale ping -c 3 <validated-peer-ip>`.
- `vpn-ui` gets a single sudoers permission for the fixed helper with no
  command-line arguments. The helper accepts the IP on stdin; no shell
  interpolation or privileged control operations.
- The server-side helper has a 16-second execution timeout; the web process
  caps simultaneous pings at four. Root helper stdout is parsed and reduces to
  RTT and route type; stderr and administrative status are not returned.
- ICMP endpoint remains separately available as
  `POST /api/tailscale/ping`.
- ICMP failure does *not* mean unavailable Tailscale: a Windows firewall may
  block it even when the control-plane ping responds through DERP.
- Since the UI has a localhost HTTP listener, continue to protect the
  Nginx/443 reverse proxy with Basic auth and never expose 8090 directly.

## Production installation

Build from feature branch against Rust 1.98; deploy the binary and three
helper artifacts using `deploy/install-tailscale-native-ping-v083.sh`.
This installer checks previous version 0.8.2, backs it up under
`/root/vpn-ui-backups`, validates sudoers and helper syntax, and restarts
**only** vpn-ui.service. On failure it attempts a rollback of both helper
permissions and the binary. No WG, AWG, Nginx or tailscaled restart.

If `vpn-ui.service` has `NoNewPrivileges=yes`, installation deliberately
refuses because the narrow sudo helper would not function. Review the
sandbox and privilege design instead of loosening it automatically.
