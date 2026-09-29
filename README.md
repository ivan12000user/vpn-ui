# VPN UI

Unified self-hosted Web UI for **WireGuard** and **AmneziaWG**.

Одна панель для двух VPN-интерфейсов: общий обзор, статусы пиров, управление клиентами и настройки интерфейсов без зависимости от отдельной машины разработчика.

## Current version

`0.7.0` — Rust / Axum application.

The application is designed to run entirely on the VPN server. Development machines, VPN peers and GitHub are not runtime dependencies.

## Implemented

- common dashboard for WireGuard and AmneziaWG;
- peer inventory for both VPN types;
- online/total counters on the overview page;
- peer status highlighting instead of a separate status column;
- responsive desktop/mobile layout;
- interface settings view;
- authenticated client configuration export;
- QR-code export for client configuration;
- peer creation and editing;
- peer enable / disable / delete operations;
- management API bridge for CRUD operations;
- read-only peer inventory API;
- health endpoint;
- compact navigation and improved refresh/ping states.

## Architecture

- Rust 2024 edition;
- Axum HTTP server;
- Tokio async runtime;
- server-side integration with WireGuard / AmneziaWG configuration and live state;
- no external database or cloud service required by the application design;
- production runtime is self-contained on the VPN server.

## Build

Requirements:

- Rust `1.98+`;
- Cargo;
- Linux target environment for deployment.

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo build --release --locked
```

Result:

```text
target/release/vpn-ui
```

## Runtime

Default listen address used by the project:

```text
127.0.0.1:8090
```

It can be overridden with:

```text
VPN_UI_LISTEN
```

For production, keep the application bound to localhost or a trusted management network and publish it through a controlled reverse proxy/VPN path when remote access is needed.

## Project goal

The project is intended to replace separate WireGuard and AmneziaWG administration panels with one consistent interface while preserving the useful parts of both existing workflows.

Priorities:

- clear live status;
- minimal unnecessary background traffic;
- mobile usability;
- predictable configuration management;
- safe peer lifecycle operations;
- independence from development hosts and external services.

## Status

Active development. Version `0.7.0` already includes peer CRUD and responsive management UI, but the project should still be treated as evolving software until a dedicated public stable release is prepared.

## Security

Do not commit production private keys, client configs, passwords, session secrets or real server backups.

Before exposing the UI outside localhost/trusted VPN networks, use authentication, HTTPS and network-level access controls.

## License

MIT.

---

Keywords: WireGuard UI, AmneziaWG UI, unified VPN dashboard, self-hosted VPN, Rust VPN web UI, WireGuard admin, AmneziaWG admin.
