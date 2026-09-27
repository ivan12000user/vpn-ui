use crate::vpn::{
    amneziawg,
    common::{InterfaceStatus, PeerStatus, ProviderConfig, now_epoch},
    wireguard,
};
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
};
use serde::Serialize;

#[derive(Clone)]
pub struct AppState {
    pub wireguard: ProviderConfig,
    pub amneziawg: ProviderConfig,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    application: &'static str,
    version: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/wireguard", get(wireguard_page))
        .route("/amneziawg", get(amneziawg_page))
        .route("/api/wireguard/status", get(wireguard_api))
        .route("/api/amneziawg/status", get(amneziawg_api))
        .route("/healthz", get(health))
        .with_state(state)
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        application: "vpn-ui",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn dashboard(State(state): State<AppState>) -> impl IntoResponse {
    let (wg, awg) = tokio::join!(
        wireguard::status(&state.wireguard),
        amneziawg::status(&state.amneziawg)
    );

    let body = format!(
        r#"
<h1>VPN UI</h1>
<p class="lead">Единая панель управления WireGuard и AmneziaWG.</p>

<div class="cards">
  {}
  {}
</div>

<p class="muted">Версия {} — read-only.</p>
"#,
        dashboard_card("/wireguard", &wg),
        dashboard_card("/amneziawg", &awg),
        env!("CARGO_PKG_VERSION")
    );

    Html(layout("Обзор", &body))
}

async fn wireguard_page(State(state): State<AppState>) -> impl IntoResponse {
    let status = wireguard::status(&state.wireguard).await;
    Html(layout("WireGuard", &provider_page(&status)))
}

async fn amneziawg_page(State(state): State<AppState>) -> impl IntoResponse {
    let status = amneziawg::status(&state.amneziawg).await;
    Html(layout("AmneziaWG", &provider_page(&status)))
}

async fn wireguard_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    Json(wireguard::status(&state.wireguard).await)
}

async fn amneziawg_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    Json(amneziawg::status(&state.amneziawg).await)
}

fn dashboard_card(path: &str, status: &InterfaceStatus) -> String {
    let state_class = if status.is_online() { "ok" } else { "error" };
    let state_text = if status.is_online() {
        "ONLINE"
    } else {
        "UNAVAILABLE"
    };

    let port = status
        .listen_port
        .map(|v| format!("UDP {v}"))
        .unwrap_or_else(|| "UDP —".to_string());

    format!(
        r#"
<a class="card" href="{path}">
  <div class="card-head">
    <h2>{provider}</h2>
    <span class="badge {state_class}">{state_text}</span>
  </div>

  <div class="value">{interface}</div>
  <div class="muted">{port}</div>
  <div class="metric">{peers} пиров</div>
</a>
"#,
        provider = escape_html(&status.provider),
        interface = escape_html(&status.interface),
        peers = status.peers.len(),
    )
}

fn provider_page(status: &InterfaceStatus) -> String {
    let error = status
        .error
        .as_ref()
        .map(|err| {
            format!(
                r#"<div class="alert error">Не удалось получить состояние: {}</div>"#,
                escape_html(err)
            )
        })
        .unwrap_or_default();

    let port = status
        .listen_port
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string());

    let public_key = status
        .public_key
        .as_deref()
        .map(short_key)
        .unwrap_or_else(|| "—".to_string());

    let peers = if status.peers.is_empty() {
        r#"<div class="empty">Пиры не найдены.</div>"#.to_string()
    } else {
        peer_table(&status.peers)
    };

    format!(
        r#"
<div class="page-head">
  <div>
    <h1>{provider}</h1>
    <p class="lead">Интерфейс {interface}</p>
  </div>
</div>

{error}

<div class="summary">
  <div>
    <span>Интерфейс</span>
    <strong>{interface}</strong>
  </div>
  <div>
    <span>UDP порт</span>
    <strong>{port}</strong>
  </div>
  <div>
    <span>Public key</span>
    <strong class="mono">{public_key}</strong>
  </div>
  <div>
    <span>Пиры</span>
    <strong>{peer_count}</strong>
  </div>
</div>

<div class="panel">
  <h2>Пиры</h2>
  {peers}
</div>
"#,
        provider = escape_html(&status.provider),
        interface = escape_html(&status.interface),
        peer_count = status.peers.len(),
    )
}

fn peer_table(peers: &[PeerStatus]) -> String {
    let mut rows = String::new();

    for peer in peers {
        let (status_class, status_text) = handshake_status(peer.latest_handshake);

        rows.push_str(&format!(
            r#"
<tr>
  <td><span class="badge {status_class}">{status_text}</span></td>
  <td class="mono">{key}</td>
  <td class="mono">{allowed}</td>
  <td class="mono">{endpoint}</td>
  <td>{handshake}</td>
  <td>{rx}</td>
  <td>{tx}</td>
</tr>
"#,
            key = escape_html(&short_key(&peer.public_key)),
            allowed = escape_html(&peer.allowed_ips),
            endpoint = escape_html(peer.endpoint.as_deref().unwrap_or("—")),
            handshake = format_handshake(peer.latest_handshake),
            rx = format_bytes(peer.rx_bytes),
            tx = format_bytes(peer.tx_bytes),
        ));
    }

    format!(
        r#"
<div class="table-wrap">
<table>
<thead>
<tr>
  <th>Статус</th>
  <th>Public key</th>
  <th>VPN IP / AllowedIPs</th>
  <th>Endpoint</th>
  <th>Handshake</th>
  <th>RX</th>
  <th>TX</th>
</tr>
</thead>
<tbody>
{rows}
</tbody>
</table>
</div>
"#
    )
}

fn handshake_status(timestamp: Option<u64>) -> (&'static str, &'static str) {
    let Some(timestamp) = timestamp else {
        return ("never", "NEVER");
    };

    let age = now_epoch().saturating_sub(timestamp);

    if age <= 180 {
        ("ok", "ONLINE")
    } else if age <= 86_400 {
        ("recent", "RECENT")
    } else {
        ("offline", "OFFLINE")
    }
}

fn format_handshake(timestamp: Option<u64>) -> String {
    let Some(timestamp) = timestamp else {
        return "никогда".to_string();
    };

    let age = now_epoch().saturating_sub(timestamp);

    if age < 60 {
        format!("{age} сек назад")
    } else if age < 3_600 {
        format!("{} мин назад", age / 60)
    } else if age < 86_400 {
        format!("{} ч {} мин назад", age / 3_600, (age % 3_600) / 60)
    } else {
        format!("{} д {} ч назад", age / 86_400, (age % 86_400) / 3_600)
    }
}

fn format_bytes(value: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let value_f = value as f64;

    if value_f >= GIB {
        format!("{:.2} GiB", value_f / GIB)
    } else if value_f >= MIB {
        format!("{:.2} MiB", value_f / MIB)
    } else if value_f >= KIB {
        format!("{:.2} KiB", value_f / KIB)
    } else {
        format!("{value} B")
    }
}

fn short_key(value: &str) -> String {
    if value.chars().count() <= 18 {
        return value.to_string();
    }

    let start: String = value.chars().take(10).collect();
    let end: String = value
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    format!("{start}…{end}")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const STYLE: &str = r#"
:root {
  color-scheme: dark;
  --bg: #101418;
  --panel: #1a2027;
  --panel2: #151a20;
  --border: #303943;
  --text: #f1f4f7;
  --muted: #94a3b3;
  --accent: #68a8ff;
  --green: #5bc58b;
  --yellow: #d9b65e;
  --red: #e36c74;
}

* { box-sizing: border-box; }

body {
  margin: 0;
  font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: var(--bg);
  color: var(--text);
}

.layout {
  min-height: 100vh;
  display: grid;
  grid-template-columns: 230px 1fr;
}

nav {
  border-right: 1px solid var(--border);
  padding: 26px 17px;
}

.brand {
  font-size: 20px;
  font-weight: 750;
  margin: 0 9px 30px;
  letter-spacing: .04em;
}

nav a {
  display: block;
  color: var(--text);
  text-decoration: none;
  padding: 11px 12px;
  margin-bottom: 6px;
  border-radius: 8px;
}

nav a:hover { background: var(--panel); }

main {
  padding: 38px;
  width: 100%;
  max-width: 1500px;
}

h1 {
  margin-top: 0;
  font-size: 32px;
}

h2 { margin-top: 0; }

.lead {
  color: var(--muted);
  margin-bottom: 30px;
}

.cards {
  display: grid;
  grid-template-columns: repeat(auto-fit,minmax(300px,1fr));
  gap: 18px;
  margin-bottom: 32px;
}

.card,
.panel,
.summary {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 12px;
}

.card {
  padding: 22px;
  text-decoration: none;
  color: var(--text);
}

.card:hover { border-color: var(--accent); }

.card-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 15px;
}

.card-head h2 { margin-bottom: 0; }

.value {
  margin-top: 30px;
  font-size: 27px;
  font-weight: 750;
}

.metric {
  margin-top: 20px;
  font-size: 15px;
}

.muted { color: var(--muted); }

.summary {
  display: grid;
  grid-template-columns: repeat(4,minmax(150px,1fr));
  margin-bottom: 18px;
  overflow: hidden;
}

.summary div {
  padding: 17px 20px;
  border-right: 1px solid var(--border);
}

.summary div:last-child { border-right: 0; }

.summary span {
  display: block;
  color: var(--muted);
  font-size: 13px;
  margin-bottom: 7px;
}

.summary strong {
  display: block;
  font-size: 17px;
}

.panel {
  padding: 21px;
  margin-bottom: 18px;
}

.badge {
  display: inline-block;
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 4px 8px;
  font-size: 11px;
  font-weight: 750;
  letter-spacing: .035em;
}

.badge.ok { color: var(--green); border-color: #315e47; }
.badge.recent { color: var(--yellow); border-color: #64552e; }
.badge.offline,
.badge.error { color: var(--red); border-color: #69373d; }
.badge.never { color: var(--muted); }

.alert {
  padding: 13px 16px;
  border-radius: 8px;
  margin-bottom: 18px;
}

.alert.error {
  border: 1px solid #69373d;
  background: #27181b;
  color: #f0a2a7;
}

.table-wrap {
  overflow-x: auto;
}

table {
  border-collapse: collapse;
  width: 100%;
}

th,
td {
  border-bottom: 1px solid var(--border);
  padding: 13px 11px;
  text-align: left;
  white-space: nowrap;
}

th {
  color: var(--muted);
  font-weight: 600;
  font-size: 13px;
}

td { font-size: 14px; }

tbody tr:hover { background: var(--panel2); }

.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.empty {
  color: var(--muted);
  padding: 10px 0;
}

@media (max-width: 800px) {
  .layout { grid-template-columns: 1fr; }

  nav {
    border-right: 0;
    border-bottom: 1px solid var(--border);
  }

  main { padding: 22px; }

  .summary { grid-template-columns: 1fr 1fr; }

  .summary div {
    border-bottom: 1px solid var(--border);
  }
}
"#;

fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} — VPN UI</title>
<style>{style}</style>
</head>
<body>
<div class="layout">
<nav>
  <div class="brand">VPN UI</div>
  <a href="/">Обзор</a>
  <a href="/wireguard">WireGuard</a>
  <a href="/amneziawg">AmneziaWG</a>
</nav>
<main>{body}</main>
</div>
</body>
</html>"#,
        title = escape_html(title),
        style = STYLE,
    )
}
