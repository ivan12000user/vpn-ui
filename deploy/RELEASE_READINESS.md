# v0.8.0 deployment inventory and release gates

**Status: release candidate, not an approved clean-server installer.**

This file inventories scripts collected from the production VPN UI deployment
and the prerequisites that must be verified before any installation. It contains
no production keys, client records, token material or server configuration.

## Packaged helper layout

Files under `deploy/bin/` are installed as executable commands in
`/usr/local/bin/`:

- WG: `vpn-ui-wg-ro`, `vpn-ui-wg-meta`, `vpn-ui-wg-settings`,
  `vpn-ui-wg-client-config`, `vpn-ui-manage-wg`.
- AWG 2.0: `vpn-ui-awg-ro`, `vpn-ui-awg-meta`, `vpn-ui-awg-settings`,
  `vpn-ui-awg-client-config`, `vpn-ui-manage-awg`.
- AWG 3.1: `vpn-ui-awg31-meta`, `vpn-ui-awg31-status`,
  `vpn-ui-awg31-settings`, `vpn-ui-awg31-client-config`,
  `vpn-ui-manage-awg31`.

Files under `deploy/libexec/` go to `/usr/local/libexec/`:

- `vpn-ui-privileged`: restricted read-only WG/AWG 2.0 operations.
- `vpn-ui-manage` and `vpn-ui-manage-core-v070`: guarded peer management.
- `vpn-ui-client-config`: WG/AWG 2.0 profile generation.
- `vpn-ui-awg31-status`, `vpn-ui-awg31-settings`,
  `vpn-ui-awg31-client-config`, `vpn-ui-awg31-net`: isolated AWG 3.1 helpers.

The copied helper files have executable mode `0755`. Review owner/group and
write permissions before using privileged entry points in a new deployment.

## Minimal privileged commands

The following active sudoers source files are tracked under `deploy/sudoers/`:

- `vpn-ui`: six narrowly scoped WG/AWG 2.0 status, metadata and settings commands.
- `vpn-ui-manage`: two peer-management commands, one per legacy provider.
- `vpn-ui-client-config`: WG and AWG 2.0 profile exports.
- `vpn-ui-awg31`: AWG 3.1 status, management, settings and profile export.

Install sudoers only after checking *every* entry with `visudo -cf`. Production
locations are `/etc/sudoers.d/<name>`, owned by root with mode `0440`.
Never copy historical sudoers backups: a backup was found on the VPS but is
**not** part of this repository.

Wrappers are not interchangeable: AWG 3.1 peer inventory and runtime state
are separate from `wg0` and `awg0`. A profile export accesses private client
material only at runtime; such exports must be restricted to authenticated
administrators.

## External prerequisites (not packaged)

- The Linux x86-64 AWG 3.1 userspace engine
  `/usr/local/libexec/amneziawg-go-3.1.20260828`. An audited installed
  executable previously had SHA256
  `6177066f0e35b1cb44945136d9a10841dd3ce30fdf16009c9a815cd943a982fe`.
  Its upstream source, reproducible build and redistribution license still
  require verification. **Do not publish that binary as part of vpn-ui.**
- WireGuard and AmneziaWG executables and quick helpers, Linux routing/NAT
  setup, `sudo`, Python 3, shell tools and the relevant userspace/kernel
  capabilities.
- The base `vpn-ui.service`, `vpn-ui` system account, Nginx site credentials,
  TLS certificate/private key, and environment-specific service security
  settings require clean-install documentation and a controlled rehearsal.
  The repository currently supplies selected systemd units/drop-ins rather than
  every host-level prerequisite.
- VPN server configurations, peer inventories, private keys, client backups
  and UI access credentials are **runtime data**, never public repository files.

## Test gate

Use a trusted Linux build host or GitHub Actions, not the live VPS, for checks:

```bash
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 check --locked
cargo +1.98.0 test --locked
cargo +1.98.0 build --release --locked
python3 -m unittest discover -s tests -p 'test_awg31_*.py' -v
python3 -m unittest discover -s tests -p 'test_client_config_helpers.py' -v
python3 -m unittest discover -s tests -p 'test_readonly_deployment.py' -v
```

A passing test suite confirms source build and offline helper behavior, **not**
production compatibility or permission correctness in a newly provisioned
host. Before any actual production update, stage backups, compare generated
profiles to secret-free expectations, validate each provider, test rollback
and preserve access over an independent management path.

**Do not install or restart services automatically from this repository.**
