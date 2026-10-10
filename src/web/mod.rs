use crate::{
    geoip::{GeoIpService, enrich_status},
    tailscale,
    vpn::{
        amneziawg,
        common::{InterfaceStatus, PeerStatus, PingResult, ProviderConfig, now_epoch, ping_ip},
        wireguard,
    },
};
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::process::Command;

#[derive(Clone)]
pub struct AppState {
    pub wireguard: ProviderConfig,
    pub amneziawg: ProviderConfig,
    pub amneziawg31: ProviderConfig,
    pub ping_command: String,
    pub wg_settings_command: String,
    pub awg_settings_command: String,
    pub awg31_settings_command: String,

    pub wg_client_config_command: String,
    pub awg_client_config_command: String,
    pub awg31_client_config_command: String,

    pub wg_manage_command: String,
    pub awg_manage_command: String,
    pub awg31_manage_command: String,

    pub geoip: GeoIpService,
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

#[derive(Default)]
struct SettingsResult {
    values: HashMap<String, String>,
    error: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/wireguard", get(wireguard_page))
        .route("/amneziawg", get(amneziawg_page))
        .route("/amneziawg31", get(amneziawg31_page))
        .route("/tailscale", get(tailscale_page))
        .route("/api/tailscale/status", get(tailscale_api))
        .route("/settings", get(settings_page))
        .route("/api/admin/auth", get(admin_auth))
        .route("/admin/client", get(crate::admin::client_page))
        .route(
            "/api/admin/client-config",
            get(crate::admin::client_config_download),
        )
        .route(
            "/api/admin/wireguard/manage",
            post(crate::admin::wireguard_manage),
        )
        .route(
            "/api/admin/amneziawg/manage",
            post(crate::admin::amneziawg_manage),
        )
        .route("/api/wireguard/inventory", get(wireguard_inventory))
        .route(
            "/api/admin/amneziawg31/manage",
            post(crate::admin::amneziawg31_manage),
        )
        .route("/api/amneziawg/inventory", get(amneziawg_inventory))
        .route("/api/amneziawg31/inventory", get(amneziawg31_inventory))
        .route("/api/wireguard/status", get(wireguard_api))
        .route("/api/amneziawg/status", get(amneziawg_api))
        .route("/api/amneziawg31/status", get(amneziawg31_api))
        .route("/api/wireguard/ping", post(wireguard_ping))
        .route("/api/amneziawg/ping", post(amneziawg_ping))
        .route("/api/amneziawg31/ping", post(amneziawg31_ping))
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

async fn admin_auth() -> impl IntoResponse {
    Html(layout(
        "Управление",
        r#"
<div class="panel">
  <h1>Управление VPN</h1>
  <p class="lead">
    Авторизация подтверждена.
    Это окно можно закрыть и вернуться к списку пиров.
  </p>
</div>
"#,
    ))
}

async fn dashboard(State(state): State<AppState>) -> impl IntoResponse {
    let (wg, awg, awg31, ts) = tokio::join!(
        wireguard::status(&state.wireguard),
        amneziawg::status(&state.amneziawg),
        amneziawg::status(&state.amneziawg31),
        tailscale::status()
    );

    let body = format!(
        r#"
<h1>VPN UI</h1>
<p class="lead">Единая панель управления WireGuard и AmneziaWG.</p>

<div class="cards">
  {}
  {}
  {}
  {}
</div>

<p class="muted">Версия {}.</p>
"#,
        dashboard_card("/wireguard", &wg),
        dashboard_card("/amneziawg", &awg),
        dashboard_card("/amneziawg31", &awg31),
        tailscale_dashboard_card(&ts),
        env!("CARGO_PKG_VERSION")
    );

    Html(layout("Обзор", &body))
}

async fn wireguard_page(State(state): State<AppState>) -> impl IntoResponse {
    let mut status = wireguard::status(&state.wireguard).await;
    enrich_status(&state.geoip, &mut status).await;
    Html(layout("WireGuard", &provider_page("wireguard", &status)))
}

async fn amneziawg_page(State(state): State<AppState>) -> impl IntoResponse {
    let mut status = amneziawg::status(&state.amneziawg).await;
    enrich_status(&state.geoip, &mut status).await;
    Html(layout("AmneziaWG", &provider_page("amneziawg", &status)))
}

async fn amneziawg31_page(State(state): State<AppState>) -> impl IntoResponse {
    let mut status = amneziawg::status(&state.amneziawg31).await;
    enrich_status(&state.geoip, &mut status).await;
    Html(layout("AmneziaWG 3.1", &provider_page("amneziawg31", &status)))
}

async fn wireguard_inventory(State(state): State<AppState>) -> impl IntoResponse {
    inventory_api(&state.wg_manage_command).await
}

async fn amneziawg_inventory(State(state): State<AppState>) -> impl IntoResponse {
    inventory_api(&state.awg_manage_command).await
}

async fn amneziawg31_inventory(State(state): State<AppState>) -> impl IntoResponse {
    inventory_api(&state.awg31_manage_command).await
}

async fn inventory_api(command: &str) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    let mut child = match Command::new(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,

        Err(err) => {
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error":
                        format!(
                            "cannot execute inventory helper: {err}"
                        ),
                })),
            );
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(err) =
            tokio::io::AsyncWriteExt::write_all(&mut stdin, b"{\"op\":\"list\"}\n").await
        {
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error":
                        format!(
                            "cannot request inventory: {err}"
                        ),
                })),
            );
        }
    }

    let output = match child.wait_with_output().await {
        Ok(output) => output,

        Err(err) => {
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error":
                        format!(
                            "cannot read inventory: {err}"
                        ),
                })),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return (
            axum::http::StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "ok": false,
                "error":
                    if stderr.is_empty() {
                        "inventory helper failed"
                            .to_string()
                    } else {
                        stderr
                    },
            })),
        );
    }

    let result: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,

        Err(err) => {
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error":
                        format!(
                            "invalid inventory JSON: {err}"
                        ),
                })),
            );
        }
    };

    if result.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return (
            axum::http::StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "ok": false,
                "error":
                    "inventory helper returned failure",
            })),
        );
    }

    let Some(peers) = result.get("peers").and_then(serde_json::Value::as_array) else {
        return (
            axum::http::StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "ok": false,
                "error":
                    "inventory response has no peers array",
            })),
        );
    };

    let mut public_peers = Vec::with_capacity(peers.len());

    for peer in peers {
        let Some(public_key) = peer.get("public_key").and_then(serde_json::Value::as_str) else {
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error":
                        "inventory peer has no public_key",
                })),
            );
        };

        let name = peer.get("name").and_then(serde_json::Value::as_str);

        let vpn_ip = peer.get("vpn_ip").and_then(serde_json::Value::as_str);

        let enabled = peer
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        public_peers.push(serde_json::json!({
            "public_key": public_key,
            "name": name,
            "vpn_ip": vpn_ip,
            "enabled": enabled,
        }));
    }

    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({
            "ok": true,
            "count":
                public_peers.len(),
            "peers":
                public_peers,
        })),
    )
}

async fn wireguard_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    let mut status = wireguard::status(&state.wireguard).await;
    enrich_status(&state.geoip, &mut status).await;
    Json(status)
}

async fn amneziawg_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    let mut status = amneziawg::status(&state.amneziawg).await;
    enrich_status(&state.geoip, &mut status).await;
    Json(status)
}

async fn amneziawg31_api(State(state): State<AppState>) -> Json<InterfaceStatus> {
    let mut status = amneziawg::status(&state.amneziawg31).await;
    enrich_status(&state.geoip, &mut status).await;
    Json(status)
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

async fn amneziawg31_ping(
    State(state): State<AppState>,
    Json(request): Json<PingRequest>,
) -> Json<PingResult> {
    let status = amneziawg::status(&state.amneziawg31).await;
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


async fn tailscale_api() -> Json<tailscale::TailStatus> {
    Json(tailscale::status().await)
}

async fn tailscale_page() -> impl IntoResponse {
    let status = tailscale::status().await;
    Html(layout("Tailscale", &tailscale_page_body(&status)))
}

fn tailscale_dashboard_card(status: &tailscale::TailStatus) -> String {
    let available = status.backend_state == "Running" && status.error.is_none();
    let state_class = if available { "ok" } else { "error" };
    let state_text = if available { "ONLINE" } else { "UNAVAILABLE" };
    format!(
        r#"<a class="card" href="/tailscale">
          <div class="card-head"><h2>Tailscale</h2><span class="badge {state_class}">{state_text}</span></div>
          <div class="value">tailscale0</div>
          <div class="muted mono">{ip}</div>
          <div class="metric"><strong class="metric-online">{online}</strong> онлайн
          <span class="metric-separator">/</span><strong>{total}</strong> видимых</div>
        </a>"#,
        ip = escape_html(if status.self_ip.is_empty() { "—" } else { &status.self_ip }),
        online = status.online_count,
        total = status.peers.len(),
    )
}

fn tailscale_rows(status: &tailscale::TailStatus) -> String {
    let mut rows = String::new();
    for peer in &status.peers {
        let state = if peer.online { "ONLINE" } else { "OFFLINE" };
        let state_class = if peer.online { "ok" } else { "offline" };
        let traffic = if peer.active { "Активен" } else { "Нет обмена" };
        let exit = if peer.exit_node_option { "Да" } else { "—" };
        rows.push_str(&format!(
            r#"<tr class="status-{state_class}">
                <td class="peer-name-cell" data-label="Имя"><strong class="peer-name">{name}</strong></td>
                <td class="mono" data-label="Tailscale IP">{ip}</td>
                <td data-label="Статус">{state}</td>
                <td data-label="Обмен">{traffic}</td>
                <td data-label="Exit-node">{exit}</td>
                <td data-label="Relay">{relay}</td>
                <td data-label="Владелец">{owner}</td>
               </tr>"#,
            name = escape_html(&peer.hostname),
            ip = escape_html(&peer.ip),
            relay = escape_html(if peer.relay.is_empty() { "—" } else { &peer.relay }),
            owner = escape_html(if peer.owner.is_empty() { "—" } else { &peer.owner }),
        ));
    }
    rows
}

fn tailscale_page_body(status: &tailscale::TailStatus) -> String {
    let error = status.error.as_deref().unwrap_or("");
    let error_html = if error.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="alert error" id="ts-initial-error">{}</div>"#, escape_html(error))
    };
    let status_name = if status.backend_state.is_empty() { "—" } else { &status.backend_state };
    let hostname = if status.self_hostname.is_empty() { "—" } else { &status.self_hostname };
    let ip = if status.self_ip.is_empty() { "—" } else { &status.self_ip };
    let exit = if status.self_exit_node_option { "Предлагается" } else { "Нет" };
    format!(
        r#"<div class="page-head">
          <div><h1>Tailscale</h1><p class="lead">Локально видимые участники сети (только просмотр)</p></div>
          <div class="toolbar">
            <button id="ts-refresh" class="button" type="button">↻ Обновить</button>
            <input id="ts-filter" class="filter-input" type="search" placeholder="Поиск..." autocomplete="off">
            <label class="auto-refresh"><input id="ts-auto" type="checkbox" checked> Автообновление</label>
            <span id="ts-updated" class="muted">—</span>
          </div>
        </div>
        <div id="ts-error">{error_html}</div>
        <div class="summary">
          <div><span>Устройство</span><strong id="ts-self">{hostname}</strong></div>
          <div><span>Tailscale IP</span><strong id="ts-ip">{ip}</strong></div>
          <div><span>Видно / онлайн</span><strong id="ts-count">{total} / {online}</strong></div>
          <div><span>Exit-node / статус</span><strong id="ts-exit">{exit} · {backend}</strong></div>
        </div>
        <div class="panel">
          <h2>Участники Tailscale</h2>
          <div class="table-wrap tailscale-table">
            <table>
              <thead><tr><th>Имя</th><th>Tailscale IP</th><th>Статус</th><th>Обмен</th><th>Exit-node</th><th>Relay</th><th>Владелец</th></tr></thead>
              <tbody id="tailscale-rows">{rows}</tbody>
            </table>
          </div>
          <p class="muted" style="margin-top:16px">
            Показаны только узлы, видимые этому VPS. Список приглашённых пользователей,
            которым расшарен exit-node, сюда не входит; он требует административных данных Tailscale.
          </p>
        </div>"#,
        hostname = escape_html(hostname),
        ip = escape_html(ip),
        total = status.peers.len(),
        online = status.online_count,
        exit = exit,
        backend = escape_html(status_name),
        rows = tailscale_rows(status),
    )
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

    let online = status
        .peers
        .iter()
        .filter(|peer| handshake_status(peer.latest_handshake).0 == "ok")
        .count();

    format!(
        r#"
<a class="card" href="{path}">
  <div class="card-head">
    <h2>{provider}</h2>
    <span class="badge {state_class}">{state_text}</span>
  </div>
  <div class="value">{interface}</div>
  <div class="muted">{port}</div>
  <div class="metric">
    <strong class="metric-online">{online}</strong> онлайн
    <span class="metric-separator">/</span>
    <strong>{peers}</strong> всего
  </div>
</a>
"#,
        provider = escape_html(&status.provider),
        interface = escape_html(&status.interface),
        online = online,
        peers = status.peers.len(),
    )
}

async fn read_settings(command: &str) -> SettingsResult {
    let output = match Command::new(command).output().await {
        Ok(output) => output,
        Err(err) => {
            return SettingsResult {
                values: HashMap::new(),
                error: Some(format!("cannot execute {command}: {err}")),
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return SettingsResult {
            values: HashMap::new(),
            error: Some(if stderr.is_empty() {
                format!("{command} returned {}", output.status)
            } else {
                stderr
            }),
        };
    }

    let mut values = HashMap::new();

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.splitn(2, '\t');

        let key = fields.next().unwrap_or_default().trim();
        let value = fields.next().unwrap_or_default().trim();

        if !key.is_empty() {
            values.insert(key.to_string(), value.to_string());
        }
    }

    SettingsResult {
        values,
        error: None,
    }
}

async fn settings_page(State(state): State<AppState>) -> impl IntoResponse {
    let (wg, awg) = tokio::join!(
        read_settings(&state.wg_settings_command),
        read_settings(&state.awg_settings_command)
    );

    let wg_fields = [
        ("Address", "Адрес интерфейса"),
        ("ListenPort", "UDP порт"),
        ("MTU", "MTU"),
        ("Table", "Таблица маршрутизации"),
    ];

    let awg_fields = [
        ("Address", "Адрес интерфейса"),
        ("ListenPort", "UDP порт"),
        ("Jc", "Jc"),
        ("Jmin", "Jmin"),
        ("Jmax", "Jmax"),
        ("S1", "S1"),
        ("S2", "S2"),
        ("S3", "S3"),
        ("S4", "S4"),
        ("H1", "H1"),
        ("H2", "H2"),
        ("H3", "H3"),
        ("H4", "H4"),
    ];

    let body = format!(
        r#"
<div class="page-head settings-head">
  <div>
    <h1>Настройки</h1>
    <p class="lead">
      Основные параметры интерфейсов WireGuard и AmneziaWG.
    </p>
  </div>
</div>

<div class="settings-note">
  На текущем этапе параметры доступны только для просмотра.
  Изменение будет выполняться через backup, проверку и автоматический rollback.
</div>

<div class="settings-grid">
  {wg}
  {awg}
</div>
"#,
        wg = settings_panel("WireGuard", &state.wireguard.interface, &wg, &wg_fields),
        awg = settings_panel("AmneziaWG", &state.amneziawg.interface, &awg, &awg_fields),
    );

    Html(layout("Настройки", &body))
}

fn settings_panel(
    title: &str,
    interface: &str,
    settings: &SettingsResult,
    fields: &[(&str, &str)],
) -> String {
    let error = settings
        .error
        .as_ref()
        .map(|value| {
            format!(
                r#"<div class="alert error">Не удалось прочитать настройки: {}</div>"#,
                escape_html(value)
            )
        })
        .unwrap_or_default();

    let mut rows = String::new();

    for (key, label) in fields {
        let value = settings.values.get(*key).map(String::as_str).unwrap_or("—");

        rows.push_str(&format!(
            r#"
<div class="setting-row">
  <div class="setting-label">
    <span>{label}</span>
    <small>{key}</small>
  </div>
  <div class="setting-value mono">{value}</div>
</div>
"#,
            label = escape_html(label),
            key = escape_html(key),
            value = escape_html(value),
        ));
    }

    format!(
        r#"
<section class="settings-panel">
  <div class="settings-panel-head">
    <div>
      <h2>{title}</h2>
      <div class="muted mono">{interface}</div>
    </div>
  </div>

  {error}

  <div class="settings-list">
    {rows}
  </div>
</section>
"#,
        title = escape_html(title),
        interface = escape_html(interface),
        error = error,
        rows = rows,
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

    let online_count = status
        .peers
        .iter()
        .filter(|peer| handshake_status(peer.latest_handshake).0 == "ok")
        .count();

    // Always render the table and tbody, even for an initially empty VPN.
    // Otherwise, JavaScript cannot insert the first peer without reloading.
    let peers = peer_table(provider_id, &status.peers);
    let empty_class = if status.peers.is_empty() { "" } else { " hidden" };
    let table_class = if status.peers.is_empty() { " hidden" } else { "" };

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
    <a
      id="admin-auth-button"
      class="button"
      href="/api/admin/auth"
      target="_blank"
      rel="noopener"
    >🔐 Управление</a>

    <button
      id="create-peer-button"
      class="button primary"
      type="button"
    >+ Создать пир</button>

    <button id="refresh-button" class="button" type="button">↻ Обновить</button>
    <button id="ping-all-button" class="button" type="button">Проверить все</button>
    <input
      id="peer-filter"
      class="filter-input"
      type="search"
      placeholder="Поиск..."
      autocomplete="off"
    >

    <label class="auto-refresh">
      <input id="auto-refresh" type="checkbox" checked>
      Автообновление
    </label>

    <span id="last-updated" class="muted">—</span>
  </div>
</div>

<div id="structure-warning" class="alert warning hidden">
  Обнаружено рассогласование inventory и live-состояния.
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
    <span>Пиры</span>
    <strong id="summary-peers">{peer_count}</strong>
  </div>
  <div>
    <span>ONLINE</span>
    <strong id="summary-online">{online_count}</strong>
  </div>
</div>

<div class="panel">
  <h2>Пиры</h2>
  <div id="peers-empty" class="empty{empty_class}">Пиры не найдены.</div>
  <div id="peers-table" class="peers-table-container{table_class}">
    {peers}
  </div>
</div>

<div
  id="peer-modal"
  class="modal-backdrop hidden"
  aria-hidden="true"
>
  <div
    class="modal-card"
    role="dialog"
    aria-modal="true"
    aria-labelledby="peer-modal-title"
  >
    <div class="modal-head">
      <h2 id="peer-modal-title">Пир</h2>

      <button
        id="peer-modal-close"
        class="button small"
        type="button"
        aria-label="Закрыть"
      >✕</button>
    </div>

    <form id="peer-form">
      <input
        id="peer-form-mode"
        type="hidden"
        value="create"
      >

      <input
        id="peer-public-key"
        type="hidden"
      >

      <div class="peer-form-grid">
        <label>
          <span>Имя</span>
          <input
            id="peer-name"
            type="text"
            autocomplete="off"
            required
          >
        </label>

        <label>
          <span>VPN IP</span>
          <input
            id="peer-vpn-ip"
            class="mono"
            type="text"
            autocomplete="off"
            required
          >
        </label>

        <label class="wide">
          <span>Client AllowedIPs</span>
          <textarea
            id="peer-client-allowed"
            class="mono"
            rows="3"
            required
          ></textarea>
        </label>

        <label class="wide">
          <span>Доп. Server AllowedIPs</span>
          <textarea
            id="peer-server-allowed"
            class="mono"
            rows="3"
            placeholder="Можно оставить пустым"
          ></textarea>
        </label>

        <label>
          <span>PersistentKeepalive</span>
          <input
            id="peer-keepalive"
            class="mono"
            type="text"
            autocomplete="off"
            placeholder="пусто или число"
          >
        </label>
      </div>

      <div
        id="peer-form-error"
        class="alert error hidden"
      ></div>

      <div class="modal-actions">
        <button
          id="peer-form-cancel"
          class="button"
          type="button"
        >Отмена</button>

        <button
          id="peer-form-save"
          class="button primary"
          type="submit"
        >Сохранить</button>
      </div>
    </form>
  </div>
</div>
"#,
        provider_id = provider_id,
        provider = escape_html(&status.provider),
        interface = escape_html(&status.interface),
        peer_count = status.peers.len(),
        online_count = online_count,
        empty_class = empty_class,
        table_class = table_class,
    )
}

fn peer_table(provider_id: &str, peers: &[PeerStatus]) -> String {
    let mut rows = String::new();

    for peer in peers {
        let (status_class, status_text) = handshake_status(peer.latest_handshake);

        let name = peer.name.as_deref().unwrap_or("—");
        let vpn_ip = peer.vpn_ip.as_deref().unwrap_or("—");

        rows.push_str(&format!(
            r#"
<tr class="status-{status_class}" data-peer-key="{full_key}">
  <td class="status-cell" aria-hidden="true">
    <span class="badge {status_class}">{status_text}</span>
  </td>

  <td class="peer-name-cell" data-label="Имя">
    <strong class="peer-name">{name}</strong>
  </td>

  <td class="peer-ip mono" data-label="VPN IP">{vpn_ip}</td>
  <td class="peer-endpoint mono" data-label="Endpoint">{endpoint}</td>
  <td class="peer-provider" data-label="Провайдер / ASN">{provider}</td>
  <td class="peer-location" data-label="Местоположение">{location}</td>
  <td class="peer-handshake" data-label="Handshake">{handshake}</td>
  <td class="peer-rx" data-label="RX">{rx}</td>
  <td class="peer-tx" data-label="TX">{tx}</td>

  <td class="ping-cell" data-label="Ping">
    <span class="ping-result">—</span>
    <button class="button small ping-button" type="button">Ping</button>
  </td>

  <td class="actions-cell" data-label="Действия">
    <div class="peer-actions">
      <form
        class="config-form"
        method="get"
        action="/admin/client"
        target="_blank"
      >
        <input type="hidden" name="provider" value="{provider_id}">
        <input type="hidden" name="key" value="{full_key}">
        <button class="button small config-button" type="submit">
          Конфиг / QR
        </button>
      </form>

      <button
        class="button small edit-button"
        type="button"
      >Изменить</button>

      <button
        class="button small toggle-peer-button"
        type="button"
        disabled
      >…</button>

      <button
        class="button small danger delete-button"
        type="button"
      >Удалить</button>
    </div>
  </td>
</tr>
"#,
            full_key = escape_html(&peer.public_key),
            provider_id = escape_html(provider_id),
            name = escape_html(name),
            vpn_ip = escape_html(vpn_ip),
            endpoint = escape_html(peer.endpoint.as_deref().unwrap_or("—")),
            provider = escape_html(peer.geo_provider.as_deref().unwrap_or("—")),
            location = escape_html(peer.geo_location.as_deref().unwrap_or("—")),
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
  <th>Имя</th>
  <th>VPN IP</th>
  <th>Endpoint</th>
  <th>Провайдер / ASN</th>
  <th>Местоположение</th>
  <th>Handshake</th>
  <th>RX</th>
  <th>TX</th>
  <th>Ping</th>
  <th>Действия</th>
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
        return ("never", "NO DATA");
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
  grid-template-columns: 230px minmax(0, 1fr);
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
  border: 1px solid transparent;
  border-radius: 8px;
}

.nav-link.active {
  background: var(--accent);
  color: #07111d;
  border-color: var(--accent);
  font-weight: 800;
  box-shadow: 0 0 0 1px rgba(104, 168, 255, .18);
}

@media (hover: hover) and (pointer: fine) {
  nav a:not(.active):hover {
    background: var(--panel);
    border-color: #43515f;
  }
}

main {
  padding: 32px;
  width: 100%;
  max-width: none;
  min-width: 0;
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

.toolbar .button {
  flex: 0 0 auto;
  white-space: nowrap;
}

#refresh-button {
  min-width: 104px;
}

#ping-all-button {
  min-width: 118px;
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

.metric strong {
  font-size: 18px;
}

.metric-online {
  color: var(--green);
}

.metric-separator {
  margin: 0 6px;
  color: var(--muted);
}

.muted { color: var(--muted); }

.refresh-error {
  color: var(--yellow);
}

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

.settings-note {
  margin-bottom: 18px;
  padding: 13px 16px;

  color: var(--muted);
  background: var(--panel2);

  border: 1px solid var(--border);
  border-radius: 9px;
}

.settings-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 18px;
}

.settings-panel {
  min-width: 0;

  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 12px;

  overflow: hidden;
}

.settings-panel-head {
  padding: 20px;

  background: var(--panel2);
  border-bottom: 1px solid var(--border);
}

.settings-panel-head h2 {
  margin: 0 0 5px;
}

.settings-list {
  width: 100%;
}

.setting-row {
  display: grid;
  grid-template-columns: minmax(150px, .8fr) minmax(0, 1.2fr);
  gap: 18px;

  padding: 13px 18px;

  border-bottom: 1px solid var(--border);
}

.setting-row:last-child {
  border-bottom: 0;
}

.setting-label span {
  display: block;
  font-weight: 650;
}

.setting-label small {
  display: block;
  margin-top: 3px;

  color: var(--muted);
  font-size: 11px;
}

.setting-value {
  min-width: 0;

  text-align: right;
  overflow-wrap: anywhere;
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
  width: 100%;
  min-width: 0;
}

table {
  border-collapse: collapse;
  width: 100%;
}

th,
td {
  border-bottom: 1px solid var(--border);
  padding: 11px 8px;
  text-align: left;
  white-space: nowrap;
}

.peer-endpoint {
  font-size: 13px;
}

.peer-provider,
.peer-location {
  white-space: normal;
  line-height: 1.3;
  min-width: 145px;
  max-width: 230px;
}

.peer-provider,
.peer-location,
.peer-name {
  overflow-wrap: anywhere;
}

th {
  color: var(--muted);
  font-weight: 600;
  font-size: 13px;
}

td { font-size: 14px; }

.status-cell {
  display: none !important;
}

/*
 * Статус пира показывается самой строкой.
 * Скрытый badge остаётся внутренним источником статуса для JS.
 */
tbody tr.status-ok td {
  background: rgba(91, 197, 139, .055);
}

tbody tr.status-recent td {
  background: rgba(217, 182, 94, .055);
}

tbody tr.status-offline td {
  background: rgba(227, 108, 116, .055);
}

tbody tr.status-never td {
  background: rgba(148, 163, 179, .025);
}

tbody tr.status-disabled td {
  background: rgba(148, 163, 179, .045);
  opacity: .72;
}

tbody tr.status-ok .peer-name-cell {
  box-shadow: inset 4px 0 0 var(--green);
}

tbody tr.status-recent .peer-name-cell {
  box-shadow: inset 4px 0 0 var(--yellow);
}

tbody tr.status-offline .peer-name-cell {
  box-shadow: inset 4px 0 0 var(--red);
}

tbody tr.status-never .peer-name-cell {
  box-shadow: inset 4px 0 0 var(--muted);
}

tbody tr.status-disabled .peer-name-cell {
  box-shadow: inset 4px 0 0 var(--muted);
}

@media (hover: hover) and (pointer: fine) {
  tbody tr:hover td {
    filter: brightness(1.08);
  }
}

.peer-name {
  display: block;
}

.ping-cell {
  min-width: 116px;
}

.ping-result {
  display: inline-block;
  min-width: 48px;
  margin-right: 4px;
}

.ping-result.ok {
  color: var(--green);
}

.ping-result.error {
  color: var(--red);
}

.config-cell {
  min-width: 116px;
}

.config-form {
  margin: 0;
}

.config-button {
  white-space: nowrap;
}

.button.primary {
  border-color: var(--accent);
}

.button.danger {
  color: var(--danger, #d9534f);
  border-color: var(--danger, #d9534f);
  font-weight: 700;
}

.actions-cell {
  min-width: 200px;
}

.peer-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

body.modal-open {
  overflow: hidden;
}

.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: rgba(0, 0, 0, .66);
}

.modal-backdrop.hidden {
  display: none;
}

.modal-card {
  width: min(720px, 100%);
  max-height: calc(100vh - 40px);
  overflow: auto;
  padding: 20px;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--panel);
  box-shadow: 0 20px 60px rgba(0, 0, 0, .4);
}

.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 18px;
}

.modal-head h2 {
  margin: 0;
}

.peer-form-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

.peer-form-grid label {
  display: grid;
  gap: 6px;
}

.peer-form-grid label > span {
  color: var(--muted);
  font-size: 13px;
}

.peer-form-grid .wide {
  grid-column: 1 / -1;
}

.peer-form-grid input,
.peer-form-grid textarea {
  width: 100%;
  border: 1px solid var(--border);
  border-radius: 7px;
  padding: 9px 10px;
  background: var(--panel2);
  color: var(--text);
  font: inherit;
}

.peer-form-grid textarea {
  resize: vertical;
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 18px;
}

@media (max-width: 640px) {
  .peer-form-grid {
    grid-template-columns: 1fr;
  }

  .peer-form-grid .wide {
    grid-column: auto;
  }

  .modal-backdrop {
    padding: 8px;
  }

  .modal-card {
    max-height: calc(100vh - 16px);
    padding: 14px;
  }
}

.filter-input {
  width: 180px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text);
  border-radius: 7px;
  padding: 8px 10px;
  outline: none;
}

.filter-input:focus {
  border-color: var(--accent);
}

.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.empty {
  color: var(--muted);
  padding: 10px 0;
}

/*
 * 901–1600px:
 * компактная строковая таблица для ноутбуков и HiDPI-дисплеев.
 * Действия укладываются сеткой 2x2, без горизонтальной прокрутки.
 */
@media (min-width: 901px) and (max-width: 1600px) {
  .layout {
    grid-template-columns: 200px minmax(0, 1fr);
  }

  nav {
    padding-left: 12px;
    padding-right: 12px;
  }

  main {
    padding: 22px;
  }

  .page-head {
    gap: 12px;
  }

  .panel {
    padding: 14px;
  }

  .table-wrap {
    overflow: visible;
  }

  table {
    width: 100%;
  }

  th,
  td {
    padding: 8px 5px;
  }

  th {
    font-size: 11px;
  }

  td {
    font-size: 12px;
  }

  .peer-name-cell {
    min-width: 90px;
    white-space: normal;
  }

  .peer-name {
    overflow-wrap: anywhere;
  }

  .peer-endpoint {
    font-size: 11px;
  }

  .peer-provider,
  .peer-location {
    min-width: 95px;
    max-width: 150px;
    font-size: 11px;
    line-height: 1.25;
  }

  .peer-handshake {
    white-space: normal;
    min-width: 78px;
  }

  .peer-rx,
  .peer-tx {
    font-size: 11px;
  }

  .ping-cell {
    min-width: 82px;
  }

  .ping-result {
    min-width: 32px;
    margin-right: 2px;
  }

  .actions-cell {
    min-width: 148px;
    width: 148px;
  }

  .peer-actions {
    display: grid;
    grid-template-columns:
      repeat(2, minmax(0, 1fr));
    gap: 4px;
  }

  .peer-actions .config-form {
    min-width: 0;
  }

  .peer-actions .button.small {
    width: 100%;
    min-width: 0;
    min-height: 28px;
    padding: 4px 5px;
    font-size: 10px;
    white-space: nowrap;
  }

  .peer-actions .config-button {
    width: 100%;
  }
}


/*
 * 900px и ниже:
 * таблица пиров превращается в карточки.
 */
@media (max-width: 900px) {
  main {
    padding: 26px;
  }

  .page-head {
    gap: 16px;
  }

  .panel {
    padding: 16px;
  }

  .table-wrap {
    overflow: visible;
  }

  table {
    display: block;
    width: 100%;
  }

  thead {
    display: none;
  }

  tbody {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
    width: 100%;
  }

  tbody tr[data-peer-key] {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    align-content: start;

    min-width: 0;
    padding: 14px;

    background: var(--panel2);
    border: 1px solid var(--border);
    border-left-width: 4px;
    border-radius: 10px;
  }

  tbody tr[data-peer-key].status-ok {
    background: rgba(91, 197, 139, .055);
    border-left-color: var(--green);
  }

  tbody tr[data-peer-key].status-recent {
    background: rgba(217, 182, 94, .055);
    border-left-color: var(--yellow);
  }

  tbody tr[data-peer-key].status-offline {
    background: rgba(227, 108, 116, .055);
    border-left-color: var(--red);
  }

  tbody tr[data-peer-key].status-never {
    background: var(--panel2);
    border-left-color: var(--muted);
  }

  tbody tr[data-peer-key].status-disabled {
    background: var(--panel2);
    border-left-color: var(--muted);
    opacity: .76;
  }

  tbody tr[data-peer-key] td {
    background: transparent;
  }

  tbody tr[data-peer-key]:hover {
    border-top-color: #43515f;
    border-right-color: #43515f;
    border-bottom-color: #43515f;
  }

  tbody td {
    grid-column: 1 / -1;

    display: grid;
    grid-template-columns: minmax(116px, 38%) minmax(0, 1fr);
    gap: 10px;

    min-width: 0;
    padding: 7px 0;

    border-bottom: 1px solid rgba(48, 57, 67, .65);

    white-space: normal;
    font-size: 13px;
  }

  tbody td::before {
    content: attr(data-label);

    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
    line-height: 1.35;
  }

  tbody td:last-child {
    border-bottom: 0;
  }

  /*
   * Имя является заголовком карточки.
   * Отдельного поля "Статус" больше нет.
   */
  tbody .peer-name-cell {
    grid-column: 1 / -1;

    display: flex;
    align-items: center;

    min-width: 0;
    padding: 0 0 12px;

    border-bottom: 1px solid var(--border);
    box-shadow: none !important;

    text-align: left;
  }

  tbody .peer-name-cell::before {
    display: none;
  }

  .peer-name {
    min-width: 0;
    font-size: 15px;
  }

  .peer-ip,
  .peer-endpoint,
  .peer-provider,
  .peer-location,
  .peer-handshake,
  .peer-rx,
  .peer-tx {
    min-width: 0;
    max-width: none;
    overflow-wrap: anywhere;
  }

  .ping-cell {
    min-width: 0;

    grid-template-columns:
      minmax(116px, 38%)
      minmax(48px, 1fr)
      auto;

    align-items: center;
  }

  .ping-cell::before {
    grid-column: 1;
  }

  .ping-result {
    grid-column: 2;
    min-width: 0;
    margin-right: 6px;
  }

  .ping-button {
    grid-column: 3;
    justify-self: end;
  }
}


/*
 * 900px и ниже:
 * sidebar превращается в верхнюю навигацию.
 */
@media (max-width: 900px) {
  .layout {
    display: block;
    min-height: 100vh;
  }

  nav {
    position: sticky;
    top: 0;
    z-index: 20;

    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;

    padding: 10px 12px;

    background: var(--bg);

    border-right: 0;
    border-bottom: 1px solid var(--border);
  }

  .brand {
    margin: 0 14px 0 0;
    font-size: 18px;
  }

  nav a {
    display: inline-flex;
    align-items: center;
    justify-content: center;

    margin: 0;
    padding: 8px 10px;

    font-size: 14px;
  }

  main {
    padding: 18px;
  }

  h1 {
    font-size: 28px;
  }

  .lead {
    margin-bottom: 18px;
  }

  .page-head {
    display: block;
  }

  .toolbar {
    width: 100%;
    margin: 14px 0 18px;

    display: grid;
    grid-template-columns:
      auto
      auto
      minmax(180px, 1fr);

    gap: 9px;
  }

  .filter-input {
    width: 100%;
  }

  .auto-refresh {
    align-self: center;
  }

  .summary {
    grid-template-columns: 1fr 1fr;
  }

  .summary div:nth-child(2n) {
    border-right: 0;
  }

  .summary div:nth-child(-n+2) {
    border-bottom: 1px solid var(--border);
  }

  .cards {
    grid-template-columns:
      repeat(auto-fit, minmax(260px, 1fr));
  }

  .settings-grid {
    grid-template-columns: 1fr;
  }
}


/*
 * 700px и ниже:
 * настоящий телефонный layout.
 */
@media (max-width: 700px) {
  main {
    padding: 12px;
  }

  h1 {
    margin-bottom: 6px;
    font-size: 25px;
  }

  h2 {
    font-size: 20px;
  }

  nav {
    padding: 8px;
  }

  .brand {
    margin-right: 8px;
  }

  nav a {
    padding: 8px;
    font-size: 13px;
  }

  .toolbar {
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .toolbar .button {
    width: 100%;
    min-height: 42px;
  }

  .filter-input {
    grid-column: 1 / -1;

    width: 100%;
    min-height: 42px;

    font-size: 16px;
  }

  .auto-refresh {
    grid-column: 1;
    min-height: 34px;

    display: flex;
    align-items: center;

    font-size: 13px;
  }

  #last-updated {
    grid-column: 2;

    display: flex;
    align-items: center;
    justify-content: flex-end;

    text-align: right;
    font-size: 12px;
  }

  .summary {
    margin-bottom: 12px;
  }

  .summary div {
    padding: 12px;
  }

  .summary span {
    margin-bottom: 4px;
    font-size: 11px;
  }

  .summary strong {
    font-size: 15px;
  }

  .panel {
    padding: 11px;
    border-radius: 10px;
  }

  .panel > h2 {
    margin: 4px 3px 12px;
  }

  tbody {
    grid-template-columns: 1fr;
    gap: 10px;
  }

  tbody tr[data-peer-key] {
    padding: 13px;
  }

  tbody td {
    grid-template-columns:
      108px
      minmax(0, 1fr);

    gap: 8px;

    padding: 7px 0;

    font-size: 13px;
  }

  tbody td::before {
    font-size: 11px;
  }

  .ping-cell {
    grid-template-columns:
      108px
      minmax(48px, 1fr)
      auto;
  }

  .button.small {
    min-height: 34px;
    padding: 6px 10px;
  }

  .cards {
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .card {
    padding: 17px;
  }

  .value {
    margin-top: 20px;
  }

  .setting-row {
    grid-template-columns: 1fr;
    gap: 6px;

    padding: 12px 14px;
  }

  .setting-value {
    text-align: left;
  }
}


/*
 * Узкие телефоны ~320–430px.
 */
@media (max-width: 430px) {
  .brand {
    flex-basis: 100%;

    margin: 0 0 4px;

    text-align: center;
  }

  nav a {
    flex: 1 1 calc(50% - 4px);
    min-width: 0;
  }

  .toolbar {
    gap: 7px;
  }

  .summary div {
    padding: 10px;
  }

  tbody tr[data-peer-key] {
    padding: 11px;
  }

  tbody td {
    grid-template-columns:
      96px
      minmax(0, 1fr);
  }

  .ping-cell {
    grid-template-columns:
      96px
      minmax(42px, 1fr)
      auto;
  }

  .peer-name {
    font-size: 14px;
  }

  .badge {
    padding: 4px 7px;
    font-size: 10px;
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

  const inventoryUrl =
    `/api/${provider}/inventory`;

  const manageUrl =
    `/api/admin/${provider}/manage`;

  const refreshButton = document.getElementById("refresh-button");
  const pingAllButton = document.getElementById("ping-all-button");
  const peerFilter = document.getElementById("peer-filter");
  const autoRefresh = document.getElementById("auto-refresh");
  const lastUpdated = document.getElementById("last-updated");
  const structureWarning = document.getElementById("structure-warning");

  const createPeerButton =
    document.getElementById("create-peer-button");

  const peerModal =
    document.getElementById("peer-modal");

  const peerModalClose =
    document.getElementById("peer-modal-close");

  const peerModalTitle =
    document.getElementById("peer-modal-title");

  const peerForm =
    document.getElementById("peer-form");

  const peerFormMode =
    document.getElementById("peer-form-mode");

  const peerPublicKey =
    document.getElementById("peer-public-key");

  const peerName =
    document.getElementById("peer-name");

  const peerVpnIp =
    document.getElementById("peer-vpn-ip");

  const peerClientAllowed =
    document.getElementById("peer-client-allowed");

  const peerServerAllowed =
    document.getElementById("peer-server-allowed");

  const peerKeepalive =
    document.getElementById("peer-keepalive");

  const peerFormError =
    document.getElementById("peer-form-error");

  const peerFormCancel =
    document.getElementById("peer-form-cancel");

  const peerFormSave =
    document.getElementById("peer-form-save");

  let timer = null;
  let refreshing = false;

  const savedAutoRefresh =
    localStorage.getItem("vpn-ui-auto-refresh");

  if (savedAutoRefresh !== null) {
    autoRefresh.checked = savedAutoRefresh === "1";
  }

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
        text: "NO DATA",
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

  function statusRank(peer) {
    if (peer?.enabled === false) {
      return 4;
    }

    const timestamp =
      peer?.latest_handshake;

    if (!timestamp) return 3;

    const age = Math.max(
      0,
      Math.floor(Date.now() / 1000) - timestamp
    );

    if (age <= 180) return 0;
    if (age <= 86400) return 1;
    return 2;
  }

  function sortRows(peers) {
    const tbody = document.querySelector("tbody");

    if (!tbody) return;

    const peerMap = new Map(
      peers.map(peer => [peer.public_key, peer])
    );

    const rows = Array.from(
      tbody.querySelectorAll("tr[data-peer-key]")
    );

    rows.sort((a, b) => {
      const pa = peerMap.get(a.dataset.peerKey);
      const pb = peerMap.get(b.dataset.peerKey);

      if (!pa || !pb) return 0;

      const rankDiff =
        statusRank(pa) -
        statusRank(pb);

      if (rankDiff !== 0) {
        return rankDiff;
      }

      const nameA =
        (pa.name || pa.vpn_ip || "").toLocaleLowerCase();

      const nameB =
        (pb.name || pb.vpn_ip || "").toLocaleLowerCase();

      return nameA.localeCompare(nameB, "ru");
    });

    for (const row of rows) {
      tbody.appendChild(row);
    }
  }

  function applyFilter() {
    if (!peerFilter) return;

    const query = peerFilter.value
      .trim()
      .toLocaleLowerCase("ru");

    for (const row of document.querySelectorAll(
      "tr[data-peer-key]"
    )) {
      const text = [
        row.querySelector(".peer-name")?.textContent,
        row.querySelector(".peer-ip")?.textContent,
        row.querySelector(".peer-endpoint")?.textContent,
        row.querySelector(".peer-provider")?.textContent,
        row.querySelector(".peer-location")?.textContent
      ]
        .filter(Boolean)
        .join(" ")
        .toLocaleLowerCase("ru");

      row.hidden = query !== "" && !text.includes(query);
    }
  }

  function networkListText(value) {
    if (!Array.isArray(value)) {
      return "";
    }

    return value.join("\n");
  }

  function parseNetworkList(value) {
    return value
      .split(/[\s,]+/)
      .map(item => item.trim())
      .filter(Boolean);
  }

  function setPeerFormError(message) {
    if (!peerFormError) {
      return;
    }

    const text =
      String(message || "").trim();

    peerFormError.textContent = text;

    peerFormError.classList.toggle(
      "hidden",
      text === ""
    );
  }

  function openPeerModal() {
    if (!peerModal) {
      return;
    }

    setPeerFormError("");

    peerModal.classList.remove(
      "hidden"
    );

    peerModal.setAttribute(
      "aria-hidden",
      "false"
    );

    document.body.classList.add(
      "modal-open"
    );
  }

  function closePeerModal() {
    if (!peerModal) {
      return;
    }

    peerModal.classList.add(
      "hidden"
    );

    peerModal.setAttribute(
      "aria-hidden",
      "true"
    );

    document.body.classList.remove(
      "modal-open"
    );

    setPeerFormError("");
  }

  async function adminRequest(request) {
    const response = await fetch(
      manageUrl,
      {
        method: "POST",
        headers: {
          "Content-Type":
            "application/json"
        },
        cache: "no-store",
        credentials: "same-origin",
        body: JSON.stringify(
          request
        )
      }
    );

    let data = null;

    try {
      data =
        await response.json();
    } catch (_) {
      data = null;
    }

    if (response.status === 401) {
      throw new Error(
        "Требуется авторизация. " +
        "Нажмите «🔐 Управление», " +
        "введите логин/пароль и повторите."
      );
    }

    if (!response.ok) {
      throw new Error(
        data?.error ||
        `HTTP ${response.status}`
      );
    }

    if (
      !data ||
      data.ok !== true
    ) {
      throw new Error(
        data?.error ||
        "Некорректный ответ управления"
      );
    }

    return data;
  }

  function fillPeerForm(peer) {
    peerName.value =
      peer.name || "";

    peerVpnIp.value =
      peer.vpn_ip || "";

    peerClientAllowed.value =
      networkListText(
        peer.client_allowed_ips
      );

    peerServerAllowed.value =
      networkListText(
        peer.extra_server_allowed_ips
      );

    peerKeepalive.value =
      peer.persistent_keepalive ?? "";
  }

  async function openCreatePeer() {
    if (!createPeerButton) {
      return;
    }

    const oldText =
      createPeerButton.textContent;

    createPeerButton.disabled = true;

    createPeerButton.textContent =
      "Загрузка...";

    try {
      const data =
        await adminRequest({
          op: "defaults"
        });

      peerFormMode.value =
        "create";

      peerPublicKey.value =
        "";

      peerModalTitle.textContent =
        provider === "wireguard"
          ? "Создать пир WireGuard"
          : "Создать пир AmneziaWG";

      peerFormSave.textContent =
        "Создать";

      peerName.value =
        "";

      peerVpnIp.value =
        data.vpn_ip || "";

      peerClientAllowed.value =
        networkListText(
          data.client_allowed_ips
        );

      peerServerAllowed.value =
        "";

      peerKeepalive.value =
        data.persistent_keepalive ?? "";

      openPeerModal();

      peerName.focus();
    } catch (error) {
      window.alert(
        error?.message ||
        String(error)
      );
    } finally {
      createPeerButton.disabled =
        false;

      createPeerButton.textContent =
        oldText;
    }
  }

  async function openEditPeer(
    row,
    button
  ) {
    if (!row) {
      return;
    }

    const publicKey =
      row.dataset.peerKey;

    if (!publicKey) {
      return;
    }

    const oldText =
      button?.textContent ||
      "Изменить";

    if (button) {
      button.disabled = true;

      button.textContent =
        "Загрузка...";
    }

    try {
      const data =
        await adminRequest({
          op: "get",
          public_key: publicKey
        });

      if (!data.peer) {
        throw new Error(
          "Пир не найден в inventory"
        );
      }

      peerFormMode.value =
        "edit";

      peerPublicKey.value =
        publicKey;

      peerModalTitle.textContent =
        `Изменить: ${
          data.peer.name ||
          data.peer.vpn_ip ||
          "пир"
        }`;

      peerFormSave.textContent =
        "Сохранить";

      fillPeerForm(
        data.peer
      );

      openPeerModal();

      peerName.focus();
      peerName.select();
    } catch (error) {
      window.alert(
        error?.message ||
        String(error)
      );
    } finally {
      if (button) {
        button.disabled =
          false;

        button.textContent =
          oldText;
      }
    }
  }

  async function savePeerForm(event) {
    event.preventDefault();

    const mode =
      peerFormMode.value;

    if (
      mode !== "create" &&
      mode !== "edit"
    ) {
      setPeerFormError(
        "Некорректный режим формы."
      );

      return;
    }

    const name =
      peerName.value.trim();

    const vpnIp =
      peerVpnIp.value.trim();

    const clientAllowed =
      parseNetworkList(
        peerClientAllowed.value
      );

    const extraServerAllowed =
      parseNetworkList(
        peerServerAllowed.value
      );

    const keepalive =
      peerKeepalive.value.trim();

    if (!name) {
      setPeerFormError(
        "Укажите имя пира."
      );

      peerName.focus();
      return;
    }

    if (!vpnIp) {
      setPeerFormError(
        "Укажите VPN IP."
      );

      peerVpnIp.focus();
      return;
    }

    if (
      clientAllowed.length === 0
    ) {
      setPeerFormError(
        "Client AllowedIPs не может быть пустым."
      );

      peerClientAllowed.focus();
      return;
    }

    const request = {
      op: mode,
      name,
      vpn_ip: vpnIp,
      client_allowed_ips:
        clientAllowed,
      extra_server_allowed_ips:
        extraServerAllowed,
      persistent_keepalive:
        keepalive
    };

    if (mode === "edit") {
      const publicKey =
        peerPublicKey.value.trim();

      if (!publicKey) {
        setPeerFormError(
          "Не найден public key пира."
        );

        return;
      }

      request.public_key =
        publicKey;
    }

    const oldText =
      peerFormSave.textContent;

    peerFormSave.disabled =
      true;

    peerFormSave.textContent =
      mode === "create"
        ? "Создание..."
        : "Сохранение...";

    setPeerFormError("");

    try {
      await adminRequest(
        request
      );

      closePeerModal();

      await refreshStatus();
    } catch (error) {
      setPeerFormError(
        error?.message ||
        String(error)
      );
    } finally {
      peerFormSave.disabled =
        false;

      peerFormSave.textContent =
        oldText;
    }
  }

  if (createPeerButton) {
    createPeerButton.addEventListener(
      "click",
      openCreatePeer
    );
  }

  if (peerForm) {
    peerForm.addEventListener(
      "submit",
      savePeerForm
    );
  }

  if (peerModalClose) {
    peerModalClose.addEventListener(
      "click",
      closePeerModal
    );
  }

  if (peerFormCancel) {
    peerFormCancel.addEventListener(
      "click",
      closePeerModal
    );
  }

  if (peerModal) {
    peerModal.addEventListener(
      "click",
      event => {
        if (
          event.target ===
          peerModal
        ) {
          closePeerModal();
        }
      }
    );
  }

  document.addEventListener(
    "keydown",
    event => {
      if (
        event.key === "Escape" &&
        peerModal &&
        !peerModal.classList.contains(
          "hidden"
        )
      ) {
        closePeerModal();
      }
    }
  );

  document.addEventListener(
    "click",
    event => {
      const button =
        event.target.closest(
          ".edit-button"
        );

      if (!button) {
        return;
      }

      const row =
        button.closest(
          "tr[data-peer-key]"
        );

      openEditPeer(
        row,
        button
      );
    }
  );

  document.addEventListener(
    "click",
    event => {
      const button =
        event.target.closest(
          ".toggle-peer-button"
        );

      if (!button) {
        return;
      }

      const row =
        button.closest(
          "tr[data-peer-key]"
        );

      togglePeer(
        row,
        button
      );
    }
  );

  async function togglePeer(
    row,
    button
  ) {
    if (!row || !button) {
      return;
    }

    const publicKey =
      row.dataset.peerKey;

    if (!publicKey) {
      return;
    }

    const operation =
      button.dataset.operation;

    if (
      operation !== "enable" &&
      operation !== "disable"
    ) {
      window.alert(
        "Состояние пира ещё не загружено."
      );

      return;
    }

    const oldText =
      button.textContent;

    button.disabled = true;

    button.textContent =
      operation === "disable"
        ? "Отключение..."
        : "Включение...";

    try {
      await adminRequest({
        op: operation,
        public_key: publicKey
      });

      await refreshStatus();
    } catch (error) {
      button.textContent =
        oldText;

      window.alert(
        error?.message ||
        String(error)
      );
    } finally {
      if (button.isConnected) {
        button.disabled = false;
      }
    }
  }

  document.addEventListener(
    "click",
    event => {
      const button =
        event.target.closest(
          ".delete-button"
        );

      if (!button) {
        return;
      }

      const row =
        button.closest(
          "tr[data-peer-key]"
        );

      deletePeer(
        row,
        button
      );
    }
  );

  async function deletePeer(
    row,
    button
  ) {
    if (!row || !button) {
      return;
    }

    const publicKey =
      row.dataset.peerKey;

    if (!publicKey) {
      return;
    }

    const oldText =
      button.textContent;

    button.disabled = true;

    button.textContent =
      "Проверка...";

    try {
      const data =
        await adminRequest({
          op: "get",
          public_key: publicKey
        });

      const peer =
        data.peer;

      if (!peer) {
        throw new Error(
          "Пир не найден в inventory."
        );
      }

      const name =
        String(
          peer.name || ""
        ).trim();

      const vpnIp =
        String(
          peer.vpn_ip || ""
        ).trim();

      if (!name || !vpnIp) {
        throw new Error(
          "У пира отсутствует имя или VPN IP."
        );
      }

      const confirmed =
        window.confirm(
          "Удалить пир?\n\n" +
          `Имя: ${name}\n` +
          `VPN IP: ${vpnIp}\n\n` +
          "Пир будет удалён из inventory, " +
          "конфигурации и live-состояния."
        );

      if (!confirmed) {
        return;
      }

      button.textContent =
        "Удаление...";

      await adminRequest({
        op: "delete",
        public_key: publicKey,
        confirm_name: name,
        confirm_vpn_ip: vpnIp
      });

      await refreshStatus();
    } catch (error) {
      window.alert(
        error?.message ||
        String(error)
      );
    } finally {
      if (button.isConnected) {
        button.disabled = false;

        button.textContent =
          oldText;
      }
    }
  }

  function createPeerRow(publicKey) {
    const row = document.createElement("tr");

    row.dataset.peerKey = publicKey;
    row.classList.add("status-never");

    row.innerHTML = `
<td class="status-cell" aria-hidden="true">
  <span class="badge never">NO DATA</span>
</td>

<td class="peer-name-cell" data-label="Имя">
  <strong class="peer-name">—</strong>
</td>

<td class="peer-ip mono" data-label="VPN IP">—</td>
<td class="peer-endpoint mono" data-label="Endpoint">—</td>
<td class="peer-provider" data-label="Провайдер / ASN">—</td>
<td class="peer-location" data-label="Местоположение">—</td>
<td class="peer-handshake" data-label="Handshake">никогда</td>
<td class="peer-rx" data-label="RX">0 B</td>
<td class="peer-tx" data-label="TX">0 B</td>

<td class="ping-cell" data-label="Ping">
  <span class="ping-result">—</span>
  <button class="button small ping-button" type="button">Ping</button>
</td>

<td class="actions-cell" data-label="Действия">
  <div class="peer-actions">
    <form
      class="config-form"
      method="get"
      action="/admin/client"
      target="_blank"
    >
      <input type="hidden" name="provider">
      <input type="hidden" name="key">

      <button class="button small config-button" type="submit">
        Конфиг / QR
      </button>
    </form>

    <button
      class="button small edit-button"
      type="button"
    >Изменить</button>

    <button
      class="button small toggle-peer-button"
      type="button"
      disabled
    >…</button>

    <button
      class="button small danger delete-button"
      type="button"
    >Удалить</button>
  </div>
</td>
`;

    const providerInput =
      row.querySelector(
        'input[name="provider"]'
      );

    const keyInput =
      row.querySelector(
        'input[name="key"]'
      );

    if (providerInput) {
      providerInput.value = provider;
    }

    if (keyInput) {
      keyInput.value = publicKey;
    }

    return row;
  }

  function reconcileRows(inventoryPeers) {
    const tbody = document.querySelector("tbody");

    if (!tbody) {
      return [];
    }

    const wantedKeys = new Set(
      inventoryPeers.map(
        peer => peer.public_key
      )
    );

    for (const row of Array.from(
      tbody.querySelectorAll(
        "tr[data-peer-key]"
      )
    )) {
      if (
        !wantedKeys.has(
          row.dataset.peerKey
        )
      ) {
        row.remove();
      }
    }

    const rows = new Map(
      Array.from(
        tbody.querySelectorAll(
          "tr[data-peer-key]"
        )
      ).map(row => [
        row.dataset.peerKey,
        row
      ])
    );

    for (const peer of inventoryPeers) {
      if (!rows.has(peer.public_key)) {
        const row =
          createPeerRow(
            peer.public_key
          );

        tbody.appendChild(row);

        rows.set(
          peer.public_key,
          row
        );
      }
    }

    return inventoryPeers
      .map(
        peer =>
          rows.get(peer.public_key)
      )
      .filter(Boolean);
  }

  function mergeInventoryAndLive(
    inventoryPeers,
    livePeers
  ) {
    const liveMap = new Map(
      livePeers.map(peer => [
        peer.public_key,
        peer
      ])
    );

    return inventoryPeers.map(
      item => {
        const live =
          liveMap.get(
            item.public_key
          );

        return {
          ...(live || {}),

          public_key:
            item.public_key,

          name:
            item.name ||
            live?.name ||
            null,

          vpn_ip:
            item.vpn_ip ||
            live?.vpn_ip ||
            null,

          enabled:
            item.enabled !== false,

          endpoint:
            live?.endpoint ?? null,

          geo_provider:
            live?.geo_provider ?? null,

          geo_location:
            live?.geo_location ?? null,

          latest_handshake:
            live?.latest_handshake ?? null,

          rx_bytes:
            live?.rx_bytes ?? 0,

          tx_bytes:
            live?.tx_bytes ?? 0
        };
      }
    );
  }

  function inventoryHasDrift(
    inventoryPeers,
    livePeers
  ) {
    const inventoryMap =
      new Map(
        inventoryPeers.map(
          peer => [
            peer.public_key,
            peer
          ]
        )
      );

    const liveKeys =
      new Set(
        livePeers.map(
          peer => peer.public_key
        )
      );

    for (const peer of livePeers) {
      if (
        !inventoryMap.has(
          peer.public_key
        )
      ) {
        return true;
      }
    }

    for (const peer of inventoryPeers) {
      if (
        peer.enabled !== false &&
        !liveKeys.has(
          peer.public_key
        )
      ) {
        return true;
      }
    }

    return false;
  }

  function updateRow(row, peer) {
    const disabled =
      peer.enabled === false;

    const status = disabled
      ? {
          cls: "disabled",
          text: "DISABLED",
          age: "отключён"
        }
      : handshakeInfo(
          peer.latest_handshake
        );

    row.dataset.peerEnabled =
      disabled ? "false" : "true";

    row.classList.remove(
      "status-ok",
      "status-recent",
      "status-offline",
      "status-never",
      "status-disabled"
    );

    row.classList.add(
      `status-${status.cls}`
    );

    const badge = row.querySelector(".status-cell .badge");
    badge.className = `badge ${status.cls}`;
    badge.textContent = status.text;

    row.querySelector(".peer-name").textContent =
      peer.name || "—";

    row.querySelector(".peer-ip").textContent =
      peer.vpn_ip || "—";

    row.querySelector(".peer-endpoint").textContent =
      peer.endpoint || "—";

    row.querySelector(".peer-provider").textContent =
      peer.geo_provider || "—";

    row.querySelector(".peer-location").textContent =
      peer.geo_location || "—";

    row.querySelector(".peer-handshake").textContent =
      status.age;

    row.querySelector(".peer-rx").textContent =
      disabled
        ? "—"
        : formatBytes(
            peer.rx_bytes ?? 0
          );

    row.querySelector(".peer-tx").textContent =
      disabled
        ? "—"
        : formatBytes(
            peer.tx_bytes ?? 0
          );

    const pingButton =
      row.querySelector(".ping-button");

    const pingResult =
      row.querySelector(".ping-result");

    if (pingButton) {
      pingButton.disabled =
        disabled;
    }

    if (
      disabled &&
      pingResult
    ) {
      pingResult.textContent = "—";

      pingResult.classList.remove(
        "ok",
        "error"
      );
    }

    row.dataset.peerEnabled =
      disabled
        ? "false"
        : "true";

    const toggleButton =
      row.querySelector(
        ".toggle-peer-button"
      );

    if (toggleButton) {
      toggleButton.dataset.operation =
        disabled
          ? "enable"
          : "disable";

      toggleButton.textContent =
        disabled
          ? "Включить"
          : "Отключить";

      toggleButton.disabled = false;
    }
  }

  async function refreshStatus() {
    if (refreshing) return;

    refreshing = true;

    if (refreshButton) {
      refreshButton.disabled = true;
      refreshButton.textContent =
        "↻ ...";
    }

    try {
      const [
        statusResponse,
        inventoryResponse
      ] = await Promise.all([
        fetch(
          statusUrl,
          {
            cache: "no-store"
          }
        ),

        fetch(
          inventoryUrl,
          {
            cache: "no-store"
          }
        )
      ]);

      if (!statusResponse.ok) {
        throw new Error(
          `status HTTP ${statusResponse.status}`
        );
      }

      if (!inventoryResponse.ok) {
        throw new Error(
          `inventory HTTP ${inventoryResponse.status}`
        );
      }

      const statusData =
        await statusResponse.json();

      const inventoryData =
        await inventoryResponse.json();

      if (
        inventoryData.ok !== true ||
        !Array.isArray(
          inventoryData.peers
        )
      ) {
        throw new Error(
          inventoryData.error ||
          "invalid inventory response"
        );
      }

      if (
        !Array.isArray(
          statusData.peers
        )
      ) {
        throw new Error(
          "invalid status response"
        );
      }

      const mergedPeers =
        mergeInventoryAndLive(
          inventoryData.peers,
          statusData.peers
        );

      const rows =
        reconcileRows(
          inventoryData.peers
        );

      // Handle 0 -> 1 and 1 -> 0 peers without a browser reload.
      // Both containers (including tbody) are rendered server-side.
      const hasPeers = inventoryData.peers.length > 0;
      document.getElementById("peers-empty")
        ?.classList.toggle("hidden", hasPeers);
      document.getElementById("peers-table")
        ?.classList.toggle("hidden", !hasPeers);

      const peerMap =
        new Map(
          mergedPeers.map(
            peer => [
              peer.public_key,
              peer
            ]
          )
        );

      for (const row of rows) {
        const peer =
          peerMap.get(
            row.dataset.peerKey
          );

        if (peer) {
          updateRow(
            row,
            peer
          );
        }
      }

      sortRows(mergedPeers);
      applyFilter();

      document
        .getElementById(
          "summary-peers"
        )
        .textContent =
          inventoryData.peers.length;

      document
        .getElementById(
          "summary-port"
        )
        .textContent =
          statusData.listen_port ??
          "—";

      document
        .getElementById(
          "summary-online"
        )
        .textContent =
          mergedPeers.filter(
            peer =>
              peer.enabled !== false &&
              statusRank(
                peer
              ) === 0
          ).length;

      const drift =
        inventoryHasDrift(
          inventoryData.peers,
          statusData.peers
        );

      if (drift) {
        structureWarning
          ?.classList
          .remove("hidden");
      } else {
        structureWarning
          ?.classList
          .add("hidden");
      }

      lastUpdated.textContent =
        "Обновлено " +
        new Date()
          .toLocaleTimeString();

      lastUpdated.title = "";

      lastUpdated
        .classList
        .remove(
          "refresh-error"
        );

    } catch (error) {
      const retrySeconds =
        document.hidden
          ? 60
          : 15;

      if (
        autoRefresh?.checked
      ) {
        lastUpdated.textContent =
          `Нет связи · повтор через ${retrySeconds} с`;
      } else {
        lastUpdated.textContent =
          "Нет связи";
      }

      lastUpdated.title =
        "Ошибка обновления: " +
        (
          error?.message ||
          String(error)
        );

      lastUpdated
        .classList
        .add(
          "refresh-error"
        );

    } finally {
      refreshing = false;

      if (refreshButton) {
        refreshButton.disabled =
          false;

        refreshButton.textContent =
          "↻ Обновить";
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

  peerFilter?.addEventListener("input", applyFilter);

  refreshButton?.addEventListener("click", async () => {
    await refreshStatus();
    scheduleRefresh();
  });

  autoRefresh?.addEventListener("change", () => {
    localStorage.setItem(
      "vpn-ui-auto-refresh",
      autoRefresh.checked ? "1" : "0"
    );

    scheduleRefresh();
  });

  document.addEventListener("visibilitychange", async () => {
    if (!document.hidden && autoRefresh.checked) {
      await refreshStatus();
    }

    scheduleRefresh();
  });

  async function pingRow(row) {
    const button = row.querySelector(".ping-button");
    const resultNode = row.querySelector(".ping-result");

    if (!button || !resultNode) return;

    if (
      row?.dataset.peerEnabled === "false"
    ) {
      resultNode.textContent =
        "отключён";

      resultNode.classList.remove(
        "ok",
        "error"
      );

      return;
    }

    button.disabled = true;
    resultNode.textContent = "...";
    resultNode.classList.remove("ok", "error");

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
        resultNode.classList.add("ok");
      } else {
        resultNode.textContent =
          result.error || "timeout";
        resultNode.classList.add("error");
      }

    } catch (error) {
      resultNode.textContent = "ошибка";

    } finally {
      button.disabled = false;
    }
  }

  document.addEventListener("click", async event => {
    const button = event.target.closest(".ping-button");

    if (!button) return;

    const row = button.closest("tr[data-peer-key]");

    await pingRow(row);
  });

  pingAllButton?.addEventListener("click", async () => {
    const rows = Array.from(
      document.querySelectorAll("tr[data-peer-key]")
    ).filter(
      row =>
        row.dataset.peerEnabled !== "false"
    );

    pingAllButton.disabled = true;
    pingAllButton.textContent = "Проверка...";

    try {
      // Максимум 4 ICMP-проверки одновременно.
      let completed = 0;

      for (let i = 0; i < rows.length; i += 4) {
        const batch = rows.slice(i, i + 4);

        await Promise.all(
          batch.map(row => pingRow(row))
        );

        completed += batch.length;

        pingAllButton.textContent =
          `Проверка ${completed}/${rows.length}`;
      }
    } finally {
      pingAllButton.disabled = false;
      pingAllButton.textContent = "Проверить все";
    }
  });

  lastUpdated.textContent =
    "Синхронизация...";

  refreshStatus().finally(() => {
    scheduleRefresh();
  });
})();
"#;

const TAILSCALE_SCRIPT: &str = r#"
(() => {
  const tbody = document.getElementById('tailscale-rows');
  if (!tbody) return;
  const refresh = document.getElementById('ts-refresh');
  const auto = document.getElementById('ts-auto');
  const updated = document.getElementById('ts-updated');
  const filter = document.getElementById('ts-filter');
  const errors = document.getElementById('ts-error');
  let running = false;

  const val = value => value || '—';
  function cell(row, label, value, css) {
    const td = document.createElement('td');
    td.dataset.label = label;
    if (css) td.className = css;
    td.textContent = value;
    row.appendChild(td);
  }
  function render(data) {
    tbody.replaceChildren();
    for (const peer of (data.peers || [])) {
      const tr = document.createElement('tr');
      tr.className = peer.online ? 'status-ok' : 'status-offline';
      cell(tr, 'Имя', val(peer.hostname), 'peer-name-cell');
      cell(tr, 'Tailscale IP', val(peer.ip), 'mono');
      cell(tr, 'Статус', peer.online ? 'ONLINE' : 'OFFLINE');
      cell(tr, 'Обмен', peer.active ? 'Активен' : 'Нет обмена');
      cell(tr, 'Exit-node', peer.exit_node_option ? 'Да' : '—');
      cell(tr, 'Relay', val(peer.relay));
      cell(tr, 'Владелец', val(peer.owner));
      tbody.appendChild(tr);
    }
    document.getElementById('ts-self').textContent = val(data.self_hostname);
    document.getElementById('ts-ip').textContent = val(data.self_ip);
    document.getElementById('ts-count').textContent = (data.peers || []).length + ' / ' + (data.online_count || 0);
    document.getElementById('ts-exit').textContent =
      (data.self_exit_node_option ? 'Предлагается' : 'Нет') + ' · ' + val(data.backend_state);
    errors.textContent = data.error ? 'Не удалось получить Tailscale: ' + data.error : '';
    errors.className = data.error ? 'alert error' : '';
    filterRows();
  }
  function filterRows() {
    const query = filter.value.trim().toLocaleLowerCase('ru');
    for (const tr of tbody.rows) {
      tr.hidden = query !== '' && !tr.textContent.toLocaleLowerCase('ru').includes(query);
    }
  }
  async function poll() {
    if (running) return;
    running = true;
    refresh.disabled = true;
    try {
      const result = await fetch('/api/tailscale/status', {cache:'no-store'});
      if (!result.ok) throw new Error('HTTP ' + result.status);
      render(await result.json());
      updated.textContent = 'Обновлено ' + new Date().toLocaleTimeString();
    } catch (e) {
      updated.textContent = 'Нет связи';
      errors.textContent = String(e);
      errors.className = 'alert error';
    } finally {
      refresh.disabled = false;
      running = false;
    }
  }
  refresh.addEventListener('click', poll);
  filter.addEventListener('input', filterRows);
  setInterval(() => { if (auto.checked && !document.hidden) poll(); }, 15000);
})();
"#;

pub(crate) fn layout(title: &str, body: &str) -> String {
    let overview_active = if title == "Обзор" { " active" } else { "" };
    let wireguard_active = if title == "WireGuard" { " active" } else { "" };
    let amneziawg_active = if title == "AmneziaWG" { " active" } else { "" };
    let amneziawg31_active = if title == "AmneziaWG 3.1" { " active" } else { "" };
    let tailscale_active = if title == "Tailscale" { " active" } else { "" };
    let settings_active = if title == "Настройки" {
        " active"
    } else {
        ""
    };

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
  <a class="nav-link{overview_active}" href="/">Обзор</a>
  <a class="nav-link{wireguard_active}" href="/wireguard">WireGuard</a>
  <a class="nav-link{amneziawg_active}" href="/amneziawg">AmneziaWG 2.0</a>
  <a class="nav-link{amneziawg31_active}" href="/amneziawg31">AmneziaWG 3.1</a>
  <a class="nav-link{tailscale_active}" href="/tailscale">Tailscale</a>
  <a class="nav-link{settings_active}" href="/settings">Настройки</a>
</nav>
<main>{body}</main>
</div>
<script>{script}</script>
<script>{tailscale_script}</script>
</body>
</html>"#,
        title = escape_html(title),
        style = STYLE,
        script = SCRIPT,
        tailscale_script = TAILSCALE_SCRIPT,
        overview_active = overview_active,
        wireguard_active = wireguard_active,
        amneziawg_active = amneziawg_active,
        amneziawg31_active = amneziawg31_active,
        tailscale_active = tailscale_active,
        settings_active = settings_active,
    )
}

#[cfg(test)]
mod empty_peer_table_tests {
    use super::*;

    #[test]
    fn initial_empty_page_keeps_hidden_table_for_first_peer() {
        let status = InterfaceStatus {
            provider: "AmneziaWG 3.1".into(),
            interface: "awg1".into(),
            public_key: None,
            listen_port: Some(8444),
            peers: Vec::new(),
            error: None,
        };
        let page = provider_page("amneziawg31", &status);
        assert!(page.contains("id=\"peers-empty\" class=\"empty\""));
        assert!(page.contains("id=\"peers-table\" class=\"peers-table-container hidden\""));
        assert!(page.contains("<tbody>"), "tbody must exist for dynamic reconciliation");
        assert!(SCRIPT.contains("const hasPeers = inventoryData.peers.length > 0;"));
        assert!(SCRIPT.contains("?.classList.toggle(\"hidden\", hasPeers)"));
        assert!(SCRIPT.contains("?.classList.toggle(\"hidden\", !hasPeers)"));
    }

    #[test]
    fn initial_nonempty_page_hides_empty_message() {
        let status = InterfaceStatus {
            provider: "AmneziaWG 3.1".into(),
            interface: "awg1".into(),
            public_key: None,
            listen_port: Some(8444),
            peers: vec![PeerStatus {
                public_key: "TEST_PEER_PUBLIC_KEY".into(),
                name: Some("surface-awg31-test".into()),
                vpn_ip: Some("10.88.88.2".into()),
                endpoint: None,
                allowed_ips: "10.88.88.2/32".into(),
                latest_handshake: None,
                rx_bytes: 0,
                tx_bytes: 0,
                persistent_keepalive: Some(25),
                geo_provider: None,
                geo_location: None,
            }],
            error: None,
        };
        let page = provider_page("amneziawg31", &status);
        assert!(page.contains("id=\"peers-empty\" class=\"empty hidden\""));
        assert!(page.contains("id=\"peers-table\" class=\"peers-table-container\""));
        assert!(page.contains("data-peer-key=\"TEST_PEER_PUBLIC_KEY\""));
    }
}
