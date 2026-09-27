use crate::vpn::{
    amneziawg,
    common::{InterfaceStatus, PeerStatus, PingResult, ProviderConfig, now_epoch, ping_ip},
    wireguard,
};
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct AppState {
    pub wireguard: ProviderConfig,
    pub amneziawg: ProviderConfig,
    pub ping_command: String,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    application: &'static str,
    version: &'static str,
}

#[derive(Deserialize)]
struct PingRequest {
    public_key: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/wireguard", get(wireguard_page))
        .route("/amneziawg", get(amneziawg_page))
        .route("/api/wireguard/status", get(wireguard_api))
        .route("/api/amneziawg/status", get(amneziawg_api))
        .route("/api/wireguard/ping", post(wireguard_ping))
        .route("/api/amneziawg/ping", post(amneziawg_ping))
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
    Html(layout("WireGuard", &provider_page("wireguard", &status)))
}

async fn amneziawg_page(State(state): State<AppState>) -> impl IntoResponse {
    let status = amneziawg::status(&state.amneziawg).await;
    Html(layout("AmneziaWG", &provider_page("amneziawg", &status)))
}

async fn wireguard_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    Json(wireguard::status(&state.wireguard).await)
}

async fn amneziawg_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    Json(amneziawg::status(&state.amneziawg).await)
}

async fn wireguard_ping(
    State(state): State<AppState>,
    Json(request): Json<PingRequest>,
) -> Json<PingResult> {
    let status = wireguard::status(&state.wireguard).await;
    Json(ping_known_peer(&state.ping_command, &status, &request.public_key).await)
}

async fn amneziawg_ping(
    State(state): State<AppState>,
    Json(request): Json<PingRequest>,
) -> Json<PingResult> {
    let status = amneziawg::status(&state.amneziawg).await;
    Json(ping_known_peer(&state.ping_command, &status, &request.public_key).await)
}

async fn ping_known_peer(
    ping_command: &str,
    status: &InterfaceStatus,
    public_key: &str,
) -> PingResult {
    let Some(peer) = status
        .peers
        .iter()
        .find(|peer| peer.public_key == public_key)
    else {
        return PingResult {
            ok: false,
            ip: String::new(),
            avg_ms: None,
            received: 0,
            error: Some("peer not found".to_string()),
        };
    };

    let Some(ip) = peer.vpn_ip.as_deref() else {
        return PingResult {
            ok: false,
            ip: String::new(),
            avg_ms: None,
            received: 0,
            error: Some("peer has no valid VPN IP".to_string()),
        };
    };

    ping_ip(ping_command, ip).await
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

fn provider_page(provider_id: &str, status: &InterfaceStatus) -> String {
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
<script>
window.VPN_UI_PROVIDER = "{provider_id}";
</script>

<div class="page-head">
  <div>
    <h1>{provider}</h1>
    <p class="lead">Интерфейс {interface}</p>
  </div>

  <div class="toolbar">
    <button id="refresh-button" class="button" type="button">↻ Обновить</button>

    <label class="auto-refresh">
      <input id="auto-refresh" type="checkbox" checked>
      Автообновление
    </label>

    <span id="last-updated" class="muted">—</span>
  </div>
</div>

<div id="structure-warning" class="alert warning hidden">
  Состав пиров изменился. Обновите страницу вручную.
</div>

{error}

<div class="summary">
  <div>
    <span>Интерфейс</span>
    <strong>{interface}</strong>
  </div>
  <div>
    <span>UDP порт</span>
    <strong id="summary-port">{port}</strong>
  </div>
  <div>
    <span>Public key</span>
    <strong class="mono">{public_key}</strong>
  </div>
  <div>
    <span>Пиры</span>
    <strong id="summary-peers">{peer_count}</strong>
  </div>
</div>

<div class="panel">
  <h2>Пиры</h2>
  {peers}
</div>
"#,
        provider_id = provider_id,
        provider = escape_html(&status.provider),
        interface = escape_html(&status.interface),
        peer_count = status.peers.len(),
    )
}

fn peer_table(peers: &[PeerStatus]) -> String {
    let mut rows = String::new();

    for peer in peers {
        let (status_class, status_text) = handshake_status(peer.latest_handshake);

        let name = peer.name.as_deref().unwrap_or("—");
        let vpn_ip = peer.vpn_ip.as_deref().unwrap_or("—");

        rows.push_str(&format!(
            r#"
<tr data-peer-key="{full_key}">
  <td class="status-cell">
    <span class="badge {status_class}">{status_text}</span>
  </td>

  <td>
    <strong class="peer-name">{name}</strong>
    <div class="peer-key mono">{short_key}</div>
  </td>

  <td class="peer-ip mono">{vpn_ip}</td>
  <td class="peer-endpoint mono">{endpoint}</td>
  <td class="peer-handshake">{handshake}</td>
  <td class="peer-rx">{rx}</td>
  <td class="peer-tx">{tx}</td>

  <td class="ping-cell">
    <span class="ping-result">—</span>
    <button class="button small ping-button" type="button">Ping</button>
  </td>
</tr>
"#,
            full_key = escape_html(&peer.public_key),
            name = escape_html(name),
            short_key = escape_html(&short_key(&peer.public_key)),
            vpn_ip = escape_html(vpn_ip),
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
  <th>Имя</th>
  <th>VPN IP</th>
  <th>Endpoint</th>
  <th>Handshake</th>
  <th>RX</th>
  <th>TX</th>
  <th>Ping</th>
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

.page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 20px;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 13px;
  flex-wrap: wrap;
}

.auto-refresh {
  color: var(--muted);
  font-size: 14px;
}

.button {
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text);
  border-radius: 7px;
  padding: 8px 11px;
  cursor: pointer;
}

.button:hover {
  border-color: var(--accent);
}

.button:disabled {
  opacity: .55;
  cursor: default;
}

.button.small {
  font-size: 12px;
  padding: 5px 8px;
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

.alert.warning {
  border: 1px solid #64552e;
  background: #282414;
  color: #e4c873;
}

.hidden { display: none; }

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

.peer-name {
  display: block;
}

.peer-key {
  color: var(--muted);
  font-size: 11px;
  margin-top: 4px;
}

.ping-cell {
  min-width: 140px;
}

.ping-result {
  display: inline-block;
  min-width: 60px;
  margin-right: 5px;
}

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

  .page-head { display: block; }

  .toolbar {
    margin-bottom: 20px;
  }

  .summary { grid-template-columns: 1fr 1fr; }

  .summary div {
    border-bottom: 1px solid var(--border);
  }
}
"#;

const SCRIPT: &str = r#"
(() => {
  const provider = window.VPN_UI_PROVIDER;

  if (!provider) {
    return;
  }

  const statusUrl = `/api/${provider}/status`;
  const pingUrl = `/api/${provider}/ping`;

  const refreshButton = document.getElementById("refresh-button");
  const autoRefresh = document.getElementById("auto-refresh");
  const lastUpdated = document.getElementById("last-updated");
  const structureWarning = document.getElementById("structure-warning");

  let timer = null;
  let refreshing = false;

  function formatBytes(value) {
    const kib = 1024;
    const mib = kib * 1024;
    const gib = mib * 1024;

    if (value >= gib) return `${(value / gib).toFixed(2)} GiB`;
    if (value >= mib) return `${(value / mib).toFixed(2)} MiB`;
    if (value >= kib) return `${(value / kib).toFixed(2)} KiB`;
    return `${value} B`;
  }

  function handshakeInfo(timestamp) {
    if (!timestamp) {
      return {
        cls: "never",
        text: "NEVER",
        age: "никогда"
      };
    }

    const age = Math.max(
      0,
      Math.floor(Date.now() / 1000) - timestamp
    );

    let state;

    if (age <= 180) {
      state = { cls: "ok", text: "ONLINE" };
    } else if (age <= 86400) {
      state = { cls: "recent", text: "RECENT" };
    } else {
      state = { cls: "offline", text: "OFFLINE" };
    }

    let text;

    if (age < 60) {
      text = `${age} сек назад`;
    } else if (age < 3600) {
      text = `${Math.floor(age / 60)} мин назад`;
    } else if (age < 86400) {
      text =
        `${Math.floor(age / 3600)} ч ` +
        `${Math.floor((age % 3600) / 60)} мин назад`;
    } else {
      text =
        `${Math.floor(age / 86400)} д ` +
        `${Math.floor((age % 86400) / 3600)} ч назад`;
    }

    return {
      ...state,
      age: text
    };
  }

  function updateRow(row, peer) {
    const status = handshakeInfo(peer.latest_handshake);

    const badge = row.querySelector(".status-cell .badge");
    badge.className = `badge ${status.cls}`;
    badge.textContent = status.text;

    row.querySelector(".peer-name").textContent =
      peer.name || "—";

    row.querySelector(".peer-ip").textContent =
      peer.vpn_ip || "—";

    row.querySelector(".peer-endpoint").textContent =
      peer.endpoint || "—";

    row.querySelector(".peer-handshake").textContent =
      status.age;

    row.querySelector(".peer-rx").textContent =
      formatBytes(peer.rx_bytes);

    row.querySelector(".peer-tx").textContent =
      formatBytes(peer.tx_bytes);
  }

  async function refreshStatus() {
    if (refreshing) return;

    refreshing = true;

    if (refreshButton) {
      refreshButton.disabled = true;
      refreshButton.textContent = "↻ ...";
    }

    try {
      const response = await fetch(statusUrl, {
        cache: "no-store"
      });

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }

      const data = await response.json();

      const rows = Array.from(
        document.querySelectorAll("tr[data-peer-key]")
      );

      const peers = new Map(
        data.peers.map(peer => [peer.public_key, peer])
      );

      for (const row of rows) {
        const peer = peers.get(row.dataset.peerKey);

        if (peer) {
          updateRow(row, peer);
        }
      }

      document.getElementById("summary-peers").textContent =
        data.peers.length;

      document.getElementById("summary-port").textContent =
        data.listen_port ?? "—";

      if (rows.length !== data.peers.length) {
        structureWarning.classList.remove("hidden");
      } else {
        structureWarning.classList.add("hidden");
      }

      lastUpdated.textContent =
        "Обновлено " + new Date().toLocaleTimeString();

    } catch (error) {
      lastUpdated.textContent =
        "Ошибка обновления: " + error.message;

    } finally {
      refreshing = false;

      if (refreshButton) {
        refreshButton.disabled = false;
        refreshButton.textContent = "↻ Обновить";
      }
    }
  }

  function scheduleRefresh() {
    clearTimeout(timer);

    if (!autoRefresh.checked) {
      return;
    }

    const delay = document.hidden ? 60000 : 15000;

    timer = setTimeout(async () => {
      await refreshStatus();
      scheduleRefresh();
    }, delay);
  }

  refreshButton?.addEventListener("click", async () => {
    await refreshStatus();
    scheduleRefresh();
  });

  autoRefresh?.addEventListener("change", () => {
    scheduleRefresh();
  });

  document.addEventListener("visibilitychange", async () => {
    if (!document.hidden && autoRefresh.checked) {
      await refreshStatus();
    }

    scheduleRefresh();
  });

  document.addEventListener("click", async event => {
    const button = event.target.closest(".ping-button");

    if (!button) return;

    const row = button.closest("tr[data-peer-key]");
    const resultNode = row.querySelector(".ping-result");

    button.disabled = true;
    resultNode.textContent = "...";

    try {
      const response = await fetch(pingUrl, {
        method: "POST",
        headers: {
          "Content-Type": "application/json"
        },
        body: JSON.stringify({
          public_key: row.dataset.peerKey
        })
      });

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }

      const result = await response.json();

      if (result.ok && result.avg_ms !== null) {
        resultNode.textContent =
          `${result.avg_ms.toFixed(1)} ms`;
      } else {
        resultNode.textContent =
          result.error || "timeout";
      }

    } catch (error) {
      resultNode.textContent = "ошибка";

    } finally {
      button.disabled = false;
    }
  });

  lastUpdated.textContent =
    "Обновлено " + new Date().toLocaleTimeString();

  scheduleRefresh();
})();
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
<script>{script}</script>
</body>
</html>"#,
        title = escape_html(title),
        style = STYLE,
        script = SCRIPT,
    )
}
