use crate::web::{AppState, layout};

use axum::{
    Router,
    extract::{Request, State},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use serde_json::{Value, json};
use std::{collections::HashMap, process::Stdio};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Default)]
struct InterfaceSettingsResult {
    values: HashMap<String, String>,
    error: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/settings/manage", get(settings_manage_page))
        .route("/settings/interfaces", get(settings_interfaces_page))
        .with_state(state)
}

pub async fn redirect_legacy_settings(request: Request, next: Next) -> Response {
    if request.uri().path() == "/settings" {
        return Redirect::temporary("/settings/manage").into_response();
    }

    next.run(request).await
}

async fn settings_manage_page(State(state): State<AppState>) -> impl IntoResponse {
    let (wg, awg) = tokio::join!(
        helper_request(&state.wg_manage_command, json!({"op": "settings_get"})),
        helper_request(&state.awg_manage_command, json!({"op": "settings_get"})),
    );

    let body = format!(
        r#"
<div class="page-head settings-head">
  <div>
    <h1>Настройки</h1>
    <p class="lead">
      Значения по умолчанию для новых клиентов WireGuard и AmneziaWG.
    </p>
  </div>

  <div class="toolbar settings-toolbar">
    <a class="button primary" href="/settings/manage">Клиенты по умолчанию</a>
    <a class="button" href="/settings/interfaces">Интерфейсы · просмотр</a>
  </div>
</div>

<div class="settings-note">
  Здесь меняются только значения, используемые при создании новых пиров.
  Существующие пиры, wg0/awg0 и серверные интерфейсы не изменяются.
  Доступ ко всей панели защищается общей авторизацией при входе.
</div>

<div class="settings-grid safe-settings-grid">
  {wg}
  {awg}
</div>

<style>
.settings-toolbar {{
  justify-content: flex-end;
}}

.safe-settings-form {{
  padding: 16px 18px 18px;
}}

.safe-settings-form label {{
  display: block;
  margin-bottom: 14px;
}}

.safe-settings-form label > span {{
  display: block;
  margin-bottom: 6px;
  font-weight: 650;
}}

.safe-settings-form input,
.safe-settings-form textarea {{
  width: 100%;
  min-width: 0;
  padding: 9px 10px;
  color: var(--text);
  background: var(--panel2);
  border: 1px solid var(--border);
  border-radius: 7px;
  font: inherit;
}}

.safe-settings-form textarea {{
  min-height: 72px;
  resize: vertical;
}}

.safe-settings-form input:focus,
.safe-settings-form textarea:focus {{
  outline: none;
  border-color: var(--accent);
}}

.safe-settings-actions {{
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}}

.safe-settings-status {{
  min-height: 20px;
}}

.safe-settings-status.ok {{
  color: var(--green);
}}

.safe-settings-status.error {{
  color: var(--red);
}}

@media (max-width: 700px) {{
  .settings-toolbar,
  .settings-toolbar .button,
  .safe-settings-actions .button {{
    width: 100%;
  }}
}}
</style>

{script}
"#,
        wg = settings_form("wireguard", "WireGuard", &wg),
        awg = settings_form("amneziawg", "AmneziaWG", &awg),
        script = SAFE_SETTINGS_SCRIPT,
    );

    Html(layout("Настройки", &body))
}

async fn settings_interfaces_page(State(state): State<AppState>) -> impl IntoResponse {
    let (wg, awg) = tokio::join!(
        read_interface_settings(&state.wg_settings_command),
        read_interface_settings(&state.awg_settings_command),
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
      Текущие параметры работающих интерфейсов WireGuard и AmneziaWG.
    </p>
  </div>

  <div class="toolbar settings-toolbar">
    <a class="button" href="/settings/manage">Клиенты по умолчанию</a>
    <a class="button primary" href="/settings/interfaces">Интерфейсы · просмотр</a>
  </div>
</div>

<div class="settings-note">
  Этот раздел только для просмотра.
  Изменение Address, ListenPort, MTU и параметров AmneziaWG требует отдельного
  безопасного механизма применения и rollback и здесь пока не выполняется.
</div>

<div class="settings-grid">
  {wg}
  {awg}
</div>

<style>
.settings-toolbar {{
  justify-content: flex-end;
}}

@media (max-width: 700px) {{
  .settings-toolbar,
  .settings-toolbar .button {{
    width: 100%;
  }}
}}
</style>
"#,
        wg = interface_settings_panel("WireGuard", &state.wireguard.interface, &wg, &wg_fields,),
        awg = interface_settings_panel("AmneziaWG", &state.amneziawg.interface, &awg, &awg_fields,),
    );

    Html(layout("Настройки", &body))
}

async fn read_interface_settings(command: &str) -> InterfaceSettingsResult {
    let output = match Command::new(command).output().await {
        Ok(output) => output,
        Err(err) => {
            return InterfaceSettingsResult {
                values: HashMap::new(),
                error: Some(format!("cannot execute {command}: {err}")),
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return InterfaceSettingsResult {
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

    InterfaceSettingsResult {
        values,
        error: None,
    }
}

fn interface_settings_panel(
    title: &str,
    interface: &str,
    settings: &InterfaceSettingsResult,
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

fn settings_form(provider: &str, title: &str, result: &Result<Value, String>) -> String {
    let settings = match result {
        Ok(value) => value.get("settings").cloned().unwrap_or_else(|| json!({})),
        Err(err) => {
            return format!(
                r#"
<section class="settings-panel">
  <div class="settings-panel-head">
    <h2>{title}</h2>
  </div>
  <div class="alert error">
    Не удалось прочитать настройки: {error}
  </div>
</section>
"#,
                title = escape_html(title),
                error = escape_html(err),
            );
        }
    };

    let client_allowed = string_list(&settings, "client_allowed_ips").join(", ");
    let dns_servers = string_list(&settings, "dns_servers").join(", ");
    let endpoint = string_value(&settings, "endpoint");
    let keepalive = string_value(&settings, "persistent_keepalive");

    format!(
        r#"
<section class="settings-panel">
  <div class="settings-panel-head">
    <div>
      <h2>{title}</h2>
      <div class="muted">Значения для новых клиентов</div>
    </div>
  </div>

  <form class="safe-settings-form" data-provider="{provider}">
    <label>
      <span>Client AllowedIPs</span>
      <textarea
        class="mono"
        name="client_allowed_ips"
        required
      >{client_allowed}</textarea>
    </label>

    <label>
      <span>DNS</span>
      <textarea
        class="mono"
        name="dns_servers"
        placeholder="1.1.1.1, 1.0.0.1"
      >{dns_servers}</textarea>
    </label>

    <label>
      <span>Endpoint</span>
      <input
        class="mono"
        name="endpoint"
        type="text"
        value="{endpoint}"
        required
      >
    </label>

    <label>
      <span>Persistent Keepalive</span>
      <input
        class="mono"
        name="persistent_keepalive"
        type="number"
        min="0"
        max="65535"
        value="{keepalive}"
        placeholder="пусто = выключено"
      >
    </label>

    <div class="safe-settings-actions">
      <button class="button primary" type="submit">Сохранить</button>
      <span class="safe-settings-status muted" aria-live="polite"></span>
    </div>
  </form>
</section>
"#,
        title = escape_html(title),
        provider = escape_html(provider),
        client_allowed = escape_html(&client_allowed),
        dns_servers = escape_html(&dns_servers),
        endpoint = escape_html(&endpoint),
        keepalive = escape_html(&keepalive),
    )
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_value(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

async fn helper_request(command: &str, request: Value) -> Result<Value, String> {
    let request_bytes =
        serde_json::to_vec(&request).map_err(|err| format!("cannot encode request: {err}"))?;

    let mut child = Command::new(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cannot execute {command}: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&request_bytes)
            .await
            .map_err(|err| format!("cannot write request: {err}"))?;

        stdin
            .write_all(b"\n")
            .await
            .map_err(|err| format!("cannot finish request: {err}"))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|err| format!("cannot read result: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

        return Err(if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("{command} returned {}", output.status)
        });
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("management helper returned invalid JSON: {err}"))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const SAFE_SETTINGS_SCRIPT: &str = r#"
<script>
(() => {
  function listValue(value) {
    return value
      .split(/[\n,]+/)
      .map(item => item.trim())
      .filter(Boolean);
  }

  function setStatus(form, text, cls = "") {
    const node = form.querySelector(".safe-settings-status");
    node.textContent = text;
    node.className = `safe-settings-status ${cls || "muted"}`;
  }

  function applySettings(form, settings) {
    form.elements.client_allowed_ips.value =
      (settings.client_allowed_ips || []).join(", ");

    form.elements.dns_servers.value =
      (settings.dns_servers || []).join(", ");

    form.elements.endpoint.value = settings.endpoint || "";
    form.elements.persistent_keepalive.value =
      settings.persistent_keepalive || "";
  }

  for (const form of document.querySelectorAll(".safe-settings-form")) {
    form.addEventListener("submit", async event => {
      event.preventDefault();

      const provider = form.dataset.provider;
      const button = form.querySelector('button[type="submit"]');

      const request = {
        op: "settings_edit",
        client_allowed_ips: listValue(form.elements.client_allowed_ips.value),
        dns_servers: listValue(form.elements.dns_servers.value),
        endpoint: form.elements.endpoint.value.trim(),
        persistent_keepalive:
          form.elements.persistent_keepalive.value.trim()
      };

      button.disabled = true;
      setStatus(form, "Сохранение...");

      try {
        const response = await fetch(`/api/admin/${provider}/manage`, {
          method: "POST",
          headers: {"Content-Type": "application/json"},
          body: JSON.stringify(request)
        });

        if (response.status === 401) {
          throw new Error(
            "Авторизация истекла. Обновите страницу и войдите снова."
          );
        }

        const result = await response.json();

        if (!response.ok || !result.ok) {
          throw new Error(result.error || `HTTP ${response.status}`);
        }

        applySettings(form, result.settings || {});
        setStatus(form, "Сохранено", "ok");
      } catch (error) {
        setStatus(form, error.message || String(error), "error");
      } finally {
        button.disabled = false;
      }
    });
  }
})();
</script>
"#;
