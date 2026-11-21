# IHOR: bring VPN UI to the same v0.8.8 UI as pl-vmpico

**Target:** IHOR / `vds3081540-ihor` (`46.17.106.244` public;
Tailscale `100.88.88.1`; ZeroTier `10.55.55.1`).

**Source:** the tested `feature/v088-zerotier-readonly` branch,
including `be17574e9d326e6c06b1b36122c2ba020a0cb055`:
canonical Tailscale/MagicDNS device names; TS Ping as primary reachability
with the local Tailscale Online signal secondary; WG/AWG3.1 UI;
read-only ZeroTier; history; mobile layouts.

The existing IHOR VPN peers, server keys, AWG obfuscation settings, routes,
VPN inventory, Nginx credentials and history **must not be copied from the
old VPS**. The two servers share *code and presentation*, not identity.

## Gate 1: inspect IHOR without changing anything

Read-only `audit.sh` checks installed UI version, routes, local service
states, two VPN interface/inventory counts, and whether the read-only and
management helpers exist. It prints no private keys or authentication data.
It must run **on IHOR** (root) and its output should be reviewed before
running the installer.

```bash
bash deploy/ihor-v088/audit.sh
```

If `wg0` or `awg1` inventory records do not match persistent/live
peer counts, **do not enable write management**: first perform an audited
host-specific inventory import preserving all keys, PSKs and AllowedIPs.
The installer does not import peers or adjust helpers/sudoers. A successful
binary update cannot by itself repair missing privileged helpers.

## Gate 2: stage the exact binary already proven on pl-vmpico

The tested 64-bit static Linux UI binary built from `be17574` was
transferred to the old VPS as
`/root/vpn-ui-v088-console.x86_64`. Its verified SHA256 is:

`232bfda6d12508dd8ea07c768766cdef3529de54f544d77eb7f7d7c37ad6b41a`

If the old VPS still holds it, send only this public application binary
to IHOR, via Tailscale or ZeroTier from the old VPS:

```bash
scp /root/vpn-ui-v088-console.x86_64 root@100.88.88.1:/root/
```

On IHOR:

```bash
printf '%s  %s\n' \
  232bfda6d12508dd8ea07c768766cdef3529de54f544d77eb7f7d7c37ad6b41a \
  /root/vpn-ui-v088-console.x86_64 | sha256sum -c -
```

This avoids another cross-architecture build and does not transfer any VPN
private material. Do not commit binary copies, inventory JSON or passwords
to this public repository.

## Gate 3: update UI binary only, with automatic rollback

```bash
bash deploy/ihor-v088/install-ui.sh
```

This installer:
- refuses any target other than Tailscale IP `100.88.88.1`;
- requires the exact SHA256 above and the expected UI executable path;
- backs up the previous UI binary under `/root/vpn-ui-ihor-v088-backup-*`;
- atomically replaces only `/usr/local/bin/vpn-ui` and restarts only
  `vpn-ui.service`;
- checks version, dashboard and provider pages, local Tailscale identity,
  WG and AWG3.1 inventories, and ZeroTier API response shape;
- compares WG and AWG persistent configs, live peer public-key sets and
  protected service states before and after;
- restores the previous UI executable and restarts `vpn-ui` if any
  deployment check fails.

### Intentional exclusions

This is an update of the web app, **not an unattended full server migration**.
Do not import the old VPS's `server.json`, client JSON, VPN tunnel configs,
TLS private keys, Nginx configuration or sudoers onto IHOR. ZeroTier, native
Tailscale Ping, history collector and VPN manager functionality depends on
locally installed helpers and correct service permissions. If the audit
reports missing helpers, resolve them individually and review each sudoers
rule before enabling management; no broad sudo grant.

## Verification

After a successful update:
- `http://127.0.0.1:8090/healthz` returns `version: 0.8.8`;
- `/tailscale` shows verified reachability first and console/MagicDNS names;
- `/wireguard`, `/amneziawg31`, `/zerotier`, `/history` and
  `/settings/manage` load;
- existing WG and AWG server configs and public-key sets match their preflight
  hashes;
- only `vpn-ui.service` has been restarted.

Do not assume HTTPS public port and host URL match those on pl-vmpico:
keep IHOR's current authenticated Nginx endpoint, which may differ.
