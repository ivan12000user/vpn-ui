# VPN UI 0.8.8: ZeroTier / AWG2 UI retirement

Base: installed v0.8.7, commit `577b77067fcfd043551ae072909c8559576ceb29`.
This PR targets `feature/v087-pixel-compact-peers`, not the older master.

`/zerotier` and `/api/zerotier/status` read local node info, joined networks,
assigned addresses and known VL1 peers. CLI calls are exclusively
`-j info`, `-j listnetworks`, `-j listpeers`. There is no Central API,
join, leave, set, ping or network-configuration operation. See the
[official CLI reference](https://docs.zerotier.com/cli/).
Known peers include infrastructure nodes; they are not a complete membership
inventory and do not prove online status. Missing booleans stay unknown.
Only explicitly selected fields reach the API; raw JSON, stderr, tokens,
identity files and credentials are never returned. Each read has a 3 second
deadline and 1 MiB cap. Requests share a 10 second single-flight cache,
including errors. Reads fail independently; missing CLI, permissions and
invalid JSON are visible without hiding other successful sections.

AWG2 navigation, dashboard, settings, history selection, provider API routes,
client-config selection and management route are removed. Existing AWG2
configuration, helpers, tunnel and history data are preserved. The history
collector remains unchanged; the web response filters AWG2 entries.
WG/AWG3/Tailscale and existing mobile layout remain in place. ZeroTier cards
wrap long names and IPv6 strings; mobile navigation uses existing 44px targets.

## Installation (on the VPS, reviewed source and matching Linux release binary)

Do not run older broad deployment scripts for this update. Build and test the
reviewed branch with Rust 1.98.0 (`cargo +1.98.0 test --locked` and
`cargo +1.98.0 build --release --locked`). Transfer the reviewed checkout and
binary to a private staging directory. Verify the source commit and the
binary SHA256 independently against the build machine / CI artifact.
From that directory on the VPS:

```bash
bash -n deploy/install-zerotier-v088.sh
sudo bash deploy/install-zerotier-v088.sh VERIFIED_64_CHARACTER_BINARY_SHA256
```

The installer requires running v0.8.7, an existing `vpn-ui` service account,
and no pre-existing ZeroTier wrapper/sudoers file. It checks sudoers syntax,
locks concurrent installs, verifies the supplied binary checksum, saves the
old binary and service metadata, installs one root-owned wrapper and three
exact sudoers read permissions, atomically swaps the UI binary, then restarts
**only vpn-ui.service**. It never changes Nginx, VPN units/configs, credentials,
history database or the collector timer. It compares state, PID and activation
timestamp of the protected services before/after. A failure restores v0.8.7,
removes the new permissions/wrapper and checks rollback health.

The standard CLI path is `/usr/sbin/zerotier-cli`. Missing ZeroTier is supported:
the UI explicitly shows unavailable; this installer does not install or start it.
An existing `VPN_UI_ZEROTIER_COMMAND` override takes precedence; investigate it
if reads fail. Do not make `authtoken.secret` or `identity.secret` world-readable.
For a nonstandard CLI location, review an exact-path wrapper and sudoers rule
separately; do not grant unrestricted sudo.

Backups are printed under `/root/vpn-ui-backups/zerotier-v088-*`.
For a deliberate later rollback, replace `/usr/local/bin/vpn-ui` atomically
with the saved `vpn-ui.0.8.7`, remove only the two newly installed ZeroTier files
and restart only `vpn-ui.service`; verify `/healthz` reports 0.8.7.

## Offline validation

Rust tests cover field selection, unknown state, HTML escaping, partial failure
and rejection of mutation commands. Python integration tests use a fake CLI
and an isolated HTTP listener; installer success and rollback run against fake
OS tools in a disposable directory. Browser checks at widths 360/412/448/1280
assert no page overflow, no JavaScript errors and usable navigation, and save
screenshots. No test connects to production or creates VPN interfaces.
