# History, 24h and 7d — v0.8.4

Features:
- `/history`: individual historical charts and stats for WG, AWG2, AWG3.1,
  and Tailscale. Four-provider support reuses existing read-only status + ping APIs.
- `/api/history?hours=24|168&provider=...&peer=...` returns a bounded
  read-only SQLite series. The 24h/7d options are strictly validated;
  queries never run shell commands or accept database paths.
- `vpn-ui-history.timer`: unprivileged 5-minute sampling via localhost
  `127.0.0.1:8090`; max 16 Ping requests per run and max 4 concurrent.
- SQLite history at `/var/lib/vpn-ui/history/metrics.sqlite3`, mode 0600
  under `vpn-ui`; database retained for 14 days. No third-party SQLite
  Rust dependency; Python sqlite3 stdlib is used.
- Ping success percentage is computed ONLY among actual Ping probes.
  Missing observations aren't treated as downtime.
- Tailscale online indicator comes from tailscaled; WG/AWG "online" means
  handshake seen within 3 minutes, not continuous established connection.
- Historical collector NEVER calls manage, routes, restart or VPN mutations.
- The UI remains bound to localhost and requires existing HTTPS Nginx auth
  for external access.

Rollout: `deploy/install-history-v084.sh`, from prebuilt v0.8.3.
Staging area `/opt/vpn-ui-history-stage` contains binary, collector, unit
and timer. The guarded installer saves old binary, validates version and
service account, installs only new sampler units, restarts only vpn-ui.service,
collects one initial set of samples, verifies history and all VPN services.
On failure the installer stops only its new sampler, removes its files, restores
v0.8.3 and restarts only vpn-ui.service. History data is deliberately retained.

To disable only background collection without impacting VPN:
`systemctl disable --now vpn-ui-history.timer`.
The page can still display previously recorded history.

Known limitation: only max 16 of the peers receive an active Ping per 5-minute
sample. A rotating selection prioritizes fair coverage and avoids heavy load
on a small VPS. Unprobed peers still receive status samples.
This feature is *not* an independent outage alert when the VPS itself is down.
