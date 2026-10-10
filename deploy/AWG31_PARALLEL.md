# AWG 3.1 parallel instance — staged deployment

Status: experimental. **Never overwrite awg0 or wg0 to test awg1.**
Server: 46.17.106.244 (Debian 12). Intended network 10.88.88.0/24,
UDP 8444, interface awg1, userspace AWG Go 3.1.20260828.

This branch adds the vpn-ui `amneziawg31` provider alongside
`wireguard` and `amneziawg`. It is not a replacement for the existing
production service until built and installed after testing.

## Separate paths (no write access to awg0)

- Server: `/etc/amnezia/amneziawg/awg1.conf`
- Inventory: `/var/lib/vpn-ui/secure/amneziawg31/server.json`
- Client records: `/var/lib/vpn-ui/secure/amneziawg31/clients/*.json`
- Lock: `/run/lock/vpn-ui-amneziawg31.lock`
- Backups: `/var/lib/vpn-ui/backups/amneziawg31/`
- Userspace binary: `/usr/local/libexec/amneziawg-go-3.1.20260828`
- Service: `vpn-ui-awg31.service`
- Web paths: `/amneziawg31`, `/api/amneziawg31/status`,
  `/api/amneziawg31/ping`, `/api/amneziawg31/inventory`,
  `/api/admin/amneziawg31/manage`
- Config/QR uses existing `/admin/client?provider=amneziawg31&key=...`.

All settings for HeaderProtectionKey, S1–S4, H1–H4 and RandomTrailers
must match client and server. `RandomTrailers=on` and valid
`HeaderProtectionKey` are required before vpn-ui creates AWG1 peers.
The header key is a shared secret; never paste it into chat or commit it.

## Build/parse test, without affecting production

Use a separate git worktree based on `feature/awg31-parallel` and run:

```bash
cargo fmt --check
cargo test --locked
cargo build --release --locked
python3 - <<'PY'
import ast
from pathlib import Path
for p in list(Path("deploy/libexec").glob("*.py")) + [
    Path("deploy/libexec/vpn-ui-manage"),
    Path("deploy/libexec/vpn-ui-manage-core-v070"),
    Path("deploy/libexec/vpn-ui-awg31-client-config"),
    Path("deploy/libexec/vpn-ui-awg31-settings"),
    Path("deploy/bin/vpn-ui-awg31-meta"),
]:
    ast.parse(p.read_text(), filename=str(p))
    print("PASS", p)
PY
bash -n deploy/libexec/vpn-ui-awg31-net
sh -n deploy/bin/vpn-ui-manage-awg31
sh -n deploy/bin/vpn-ui-awg31-settings
sh -n deploy/bin/vpn-ui-awg31-client-config
```

## Manual deployment checklist (after tests)

1. Snapshot existing `/usr/local/bin/vpn-ui`, currently active systemd unit,
   and production inventory; do not expose private backups.
2. Copy newly built Rust binary atomically while the service is stopped,
   restarting **vpn-ui only**, never WG/AWG services.
3. Install AWG31-specific executables from `deploy/bin/` to
   `/usr/local/bin/` and `deploy/libexec/` to `/usr/local/libexec/`,
   root-owned, 0755. Existing management files require version-aware
   deployment. Add sudoers entries **only for the AWG31 argument** to the
   current vpn-ui service user, following the working production policy.
4. Create `awg1.conf` and `server.json` with 0600, inventory directories
   with 0700; initially no peers. Confirm `awg1` config is compatible with
   `awg-quick strip`. Set `Address=10.88.88.1/24`, `ListenPort=8444`,
   values `S1..S4 >= 12`, matching `HeaderProtectionKey`,
   `RandomTrailers=on`, `DisableCookies=off`.
5. Only then install `deploy/systemd/vpn-ui-awg31.service` and run
   `systemctl start vpn-ui-awg31`. Verify `awg show awg1`,
   `ip route get 10.88.88.2`, NAT/FORWARD, service logs.
6. Open UDP 8444 in the provider firewall if necessary.
7. Create a Surface test peer in vpn-ui, download its config/QR over
   authenticated HTTPS. Verify handshake, bidirectional ping and internet.
8. Reboot-test autostart while retaining independent access via
   existing awg0 / wg0 / Tailscale.

Do not delete `awg0` while any peers or fallbacks still use it.
After migration, leave the new tunnel named `awg1`; promoting it to
"primary" in the UI need not rename Linux interfaces.

## Security notes

The client-config exporter is privileged: it reads client private keys
and a shared HeaderProtectionKey and is intended to be invoked only by
an authenticated UI through a narrowly scoped sudoers rule. Production
Nginx authentication and HTTPS are mandatory. The settings exporter
redacts HeaderProtectionKey and public status never includes secrets.

If AWG1 has never been initialized, the new UI may show an error for
awg1 without affecting awg0 or wg0. Never enable write actions until
inventory, server config and live peers agree.

## Rollback

Rollback the vpn-ui binary and AWG31 helper files from the snapshot.
`systemctl stop vpn-ui-awg31` only affects awg1. Do not stop
`awg-quick@awg0` or `wg-quick@wg0`. The awg1 configuration and
inventory remain for forensic review and later retry.
