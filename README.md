# VPN UI

Unified self-hosted Web UI for **WireGuard, AmneziaWG 2.0, AmneziaWG 3.1** and a read-only **Tailscale** overview.

Одна панель для статусов VPN, управления клиентами и настройками интерфейсов.

## Start here

- [Repository map and branch policy](docs/REPOSITORY.md): which source to use, what is packaged, and what remains external.
- [Build and verify on Z270 WSL](docs/BUILD_Z270.md): one checkout, pinned toolchain, offline checks and build identity.
- [Deployment release gates](deploy/RELEASE_READINESS.md): prerequisites for a clean server and production changes.

The running UI's `0.7.0` label does not identify its Git commit or feature set.
AWG 3.1 and Tailscale have also been observed in a deployed build reporting
`0.7.0`. Compare source revisions and artifact checksums rather than version
labels alone.

## Version and release status

- Latest tagged release: **v0.7.0** (Rust / Axum).
- This integration branch is a **v0.8.0 release candidate**, **not a finished release**.
- AWG 3.1 and Tailscale are implemented in this source branch, but the standalone deployment bundle is **not yet fully reproducible**. See the draft release PR and the prerequisites below.

## Implemented

- Dashboard with peer inventory, live statuses and online/total counts.
- WireGuard (`wg0`) and AmneziaWG 2.0 (`awg0`) management.
- Separate AmneziaWG 3.1 (`awg1`) provider with isolated peer storage, status and client configuration export.
- Peer creation, editing, enable/disable and deletion with guarded privileged helpers.
- Client configuration and QR-code export, interface settings, responsive desktop/mobile pages.
- Read-only Tailscale peer status and overview, refreshed every 15 seconds.
- Health endpoint and authenticated management interface.

## Production model

The Rust application runs on the VPN server. Development machines, VPN peers, WSL and GitHub are **not runtime dependencies**.

Example internal VPN addressing for this deployment:

- WireGuard `wg0`: `10.66.66.0/24` (IPv4-only inside the tunnel).
- AmneziaWG 2.0 `awg0`: `10.77.77.0/24` (IPv4-only inside the tunnel).
- AmneziaWG 3.1 `awg1`: `10.88.88.0/24` (UDP 8444, separate userspace engine).
- Public VPS IPv6 is preserved for the host and Nginx. The old experimental WireGuard ULA `fd66:66:66::/64` is not part of the supported design.

The UI is normally bound to `127.0.0.1:8090` (override with `VPN_UI_LISTEN`). An authenticated Nginx HTTPS frontend is used for external access. Do not expose the privileged admin UI directly to the public internet.

## Architecture

- Rust 2024 edition, Axum and Tokio.
- Provider-specific read-only metadata, status collection, and client profile generators.
- Privileged operations through restricted `sudo` wrappers; inventory and peer files remain on the server.
- Separate `vpn-ui-awg31.service` invokes the userspace AWG 3.1 engine and network helper; it must not change `wg0` or `awg0`.
- The Tailscale integration is **read-only**. It does not manage tailnets, membership or exit-node permissions.

## Build

Requirements: Rust 1.98+, Cargo and a Linux target environment.

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo build --release --locked
```

Output: `target/release/vpn-ui`.

For the AWG 3.1 offline Python regression tests, also run the `tests/test_awg31_*.py` tests in a suitable isolated Linux environment.

## Deployment components

- `src/` — Rust web UI and API.
- `deploy/libexec/` — privileged management core, profile generation and network helpers.
- `deploy/bin/` — provider-specific commands and wrappers.
- `deploy/sudoers/` — restricted permissions required by helpers.
- `deploy/systemd/` — service units and UI drop-ins.
- `deploy/nginx/` — reverse-proxy configuration.
- `deploy/verify-production.sh` — read-only consistency audit.
- `deploy/install-local-ca.sh` — local CA certificate deployment/rotation helper.

The AWG 3.1 engine `amneziawg-go-3.1.20260828` is an **external deployment prerequisite**, not bundled in this repository. Confirm provenance, licensing and its verified checksum before provisioning; do not copy production private keys into source control.

The shared WG/AWG 2.0 client-profile generator, privileged read-only helper, ten WG/AWG 2.0 command wrappers and the active sudoers policies for all three providers are now tracked, with verified file hashes, offline regression tests and sudoers syntax checks. Historical backup sudoers were excluded. **A complete disaster-recovery deployment is not yet established:** external AWG 3.1 binary provenance, the base vpn-ui service, environment-specific server configuration and a clean installation rehearsal still require verification.

See [Deployment inventory and release gates](deploy/RELEASE_READINESS.md) before installing or upgrading.

Historical one-off migration, repair and legacy cleanup scripts remain available in Git history rather than the active deployment tree.

## Validation and operations

On a trusted administrative host:

```bash
bash deploy/verify-production.sh
```

This is a read-only production audit. Before any VPS update, also validate service/sudoers prerequisites, Rust and Python tests, profile rendering for all three protocols, Tailscale read-only API, SSH access, backups and a tested rollback. Do not overwrite production configuration or client inventories when installing binaries.

See [AWG31_PARALLEL.md](deploy/AWG31_PARALLEL.md) for AWG 3.1 isolation and rollout considerations.

## Security

Never commit private keys, VPN client configurations, secret headers, inventories, passwords, session credentials or real backups. Review scripts manually even after static secret scanning. Protect the UI with authentication, TLS and trusted network access controls.

## License

MIT.
