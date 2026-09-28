use crate::{
    vpn::{amneziawg, wireguard},
    web::{AppState, layout},
};

use axum::{
    Json,
    extract::{Query, State},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, PRAGMA},
    },
    response::{Html, IntoResponse, Response},
};

use qrcodegen::{QrCode, QrCodeEcc};
use serde::Deserialize;
use serde_json::{Value, json};

use std::process::Stdio;

use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Debug, Deserialize)]
pub struct ClientQuery {
    provider: String,
    key: String,
}

pub async fn client_page(
    State(state): State<AppState>,
    Query(query): Query<ClientQuery>,
) -> Response {
    let key = query.key.trim();

    if !valid_public_key(key) {
        return error_response(StatusCode::BAD_REQUEST, "Некорректный public key.");
    }

    let (name, vpn_ip, provider_label) = peer_identity(&state, &query.provider, key).await;

    let config = match load_client_config(&state, &query.provider, key).await {
        Ok(config) => config,

        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("Не удалось получить конфигурацию: {err}"),
            );
        }
    };

    let qr = match qr_svg(&config) {
        Ok(qr) => qr,

        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Не удалось построить QR: {err}"),
            );
        }
    };

    let body = format!(
        r#"
<div class="admin-config-page">
  <div class="page-head">
    <div>
      <h1>Конфиг клиента</h1>
      <p class="lead">{provider} · {name} · {vpn_ip}</p>
    </div>
  </div>

  <div class="settings-note">
    Эта страница содержит приватный ключ клиента.
    Не передавайте QR-код или конфигурационный файл посторонним.
  </div>

  <div class="admin-config-grid">

    <section class="settings-panel admin-qr-panel">
      <div class="settings-panel-head">
        <h2>QR-код</h2>
      </div>

      <div class="qr-wrap">
        {qr}
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-head admin-config-head">
        <div>
          <h2>Конфигурация</h2>
          <div class="muted mono">{vpn_ip}</div>
        </div>

        <form
          method="get"
          action="/api/admin/client-config"
        >
          <input
            type="hidden"
            name="provider"
            value="{provider_id}"
          >

          <input
            type="hidden"
            name="key"
            value="{public_key}"
          >

          <button
            class="button"
            type="submit"
          >
            Скачать .conf
          </button>
        </form>
      </div>

      <pre class="client-config">{config}</pre>
    </section>

  </div>
</div>

<style>
.admin-config-grid {{
  display: grid;
  grid-template-columns: minmax(300px,.7fr) minmax(420px,1.3fr);
  gap: 18px;
}}

.admin-qr-panel {{
  align-self: start;
}}

.qr-wrap {{
  padding: 24px;
  background: #fff;
}}

.qr-wrap svg {{
  display: block;
  width: min(100%, 520px);
  height: auto;
  margin: 0 auto;
}}

.admin-config-head {{
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
}}

.client-config {{
  margin: 0;
  padding: 20px;

  overflow: auto;

  color: var(--text);
  background: var(--panel2);

  font-family:
    ui-monospace,
    SFMono-Regular,
    Menlo,
    Consolas,
    monospace;

  font-size: 13px;
  line-height: 1.55;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}}

@media (max-width: 900px) {{
  .admin-config-grid {{
    grid-template-columns: 1fr;
  }}
}}

@media (max-width: 600px) {{
  .admin-config-head {{
    display: block;
  }}

  .admin-config-head form {{
    margin-top: 14px;
  }}

  .admin-config-head .button {{
    width: 100%;
    min-height: 42px;
  }}
}}
</style>
"#,
        provider = escape_html(&provider_label),
        provider_id = escape_html(&query.provider),
        name = escape_html(&name),
        vpn_ip = escape_html(&vpn_ip),
        public_key = escape_html(key),
        config = escape_html(&config),
        qr = qr,
    );

    no_store(Html(layout("Конфиг клиента", &body)).into_response())
}

pub async fn client_config_download(
    State(state): State<AppState>,
    Query(query): Query<ClientQuery>,
) -> Response {
    let key = query.key.trim();

    if !valid_public_key(key) {
        return error_response(StatusCode::BAD_REQUEST, "Некорректный public key.");
    }

    let (_, vpn_ip, _) = peer_identity(&state, &query.provider, key).await;

    let config = match load_client_config(&state, &query.provider, key).await {
        Ok(config) => config,

        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("Не удалось получить конфигурацию: {err}"),
            );
        }
    };

    let safe_ip = vpn_ip.replace([':', '/'], "_");

    let filename = format!("{}-{}.conf", query.provider, safe_ip);

    let mut response = (StatusCode::OK, config).into_response();

    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );

    let disposition = format!("attachment; filename=\"{}\"", filename);

    if let Ok(value) = HeaderValue::from_str(&disposition) {
        response.headers_mut().insert(CONTENT_DISPOSITION, value);
    }

    no_store(response)
}

async fn peer_identity(state: &AppState, provider: &str, key: &str) -> (String, String, String) {
    let status = match provider {
        "wireguard" => wireguard::status(&state.wireguard).await,

        "amneziawg" => amneziawg::status(&state.amneziawg).await,

        _ => {
            return ("—".to_string(), "—".to_string(), provider.to_string());
        }
    };

    let peer = status.peers.iter().find(|peer| peer.public_key == key);

    let name = peer
        .and_then(|peer| peer.name.clone())
        .unwrap_or_else(|| "—".to_string());

    let vpn_ip = peer
        .and_then(|peer| peer.vpn_ip.clone())
        .unwrap_or_else(|| "client".to_string());

    (name, vpn_ip, status.provider)
}

async fn load_client_config(state: &AppState, provider: &str, key: &str) -> Result<String, String> {
    let command = match provider {
        "wireguard" => &state.wg_client_config_command,

        "amneziawg" => &state.awg_client_config_command,

        _ => return Err("unknown provider".to_string()),
    };

    let mut child = Command::new(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cannot execute {command}: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(key.as_bytes())
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
        let stderr = String::from_utf8_lossy(&output.stderr);

        return Err(stderr.trim().to_string());
    }

    let config =
        String::from_utf8(output.stdout).map_err(|_| "config is not valid UTF-8".to_string())?;

    if config.trim().is_empty() {
        return Err("empty config".to_string());
    }

    Ok(config)
}

pub async fn wireguard_manage(
    State(state): State<AppState>,
    Json(request): Json<Value>,
) -> Response {
    manage_request(&state.wg_manage_command, request).await
}

pub async fn amneziawg_manage(
    State(state): State<AppState>,
    Json(request): Json<Value>,
) -> Response {
    manage_request(&state.awg_manage_command, request).await
}

async fn manage_request(command: &str, request: Value) -> Response {
    let request_bytes = match serde_json::to_vec(&request) {
        Ok(value) => value,

        Err(err) => {
            return manage_error(
                StatusCode::BAD_REQUEST,
                &format!("cannot encode request: {err}"),
            );
        }
    };

    let mut child = match Command::new(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,

        Err(err) => {
            return manage_error(
                StatusCode::BAD_GATEWAY,
                &format!("cannot execute {command}: {err}"),
            );
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(err) = stdin.write_all(&request_bytes).await {
            return manage_error(
                StatusCode::BAD_GATEWAY,
                &format!("cannot write request: {err}"),
            );
        }

        if let Err(err) = stdin.write_all(b"\n").await {
            return manage_error(
                StatusCode::BAD_GATEWAY,
                &format!("cannot finish request: {err}"),
            );
        }
    }

    let output = match child.wait_with_output().await {
        Ok(output) => output,

        Err(err) => {
            return manage_error(
                StatusCode::BAD_GATEWAY,
                &format!("cannot read result: {err}"),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("{command} returned {}", output.status)
        };

        return manage_error(StatusCode::BAD_REQUEST, &message);
    }

    let result: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,

        Err(err) => {
            return manage_error(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "management helper returned \
invalid JSON: {err}"
                ),
            );
        }
    };

    no_store(Json(result).into_response())
}

fn manage_error(status: StatusCode, message: &str) -> Response {
    no_store(
        (
            status,
            Json(json!({
                "ok": false,
                "error": message,
            })),
        )
            .into_response(),
    )
}

fn valid_public_key(value: &str) -> bool {
    value.len() == 44
        && value.ends_with('=')
        && value[..43]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/')
}

fn qr_svg(text: &str) -> Result<String, String> {
    let qr =
        QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|_| "data too long".to_string())?;

    let border = 4;
    let size = qr.size();
    let view_size = size + border * 2;

    let mut path = String::new();

    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{} {}h1v1h-1z", x + border, y + border));
            }
        }
    }

    Ok(format!(
        r##"<svg
xmlns="http://www.w3.org/2000/svg"
viewBox="0 0 {0} {0}"
shape-rendering="crispEdges"
aria-label="QR config"
>
<rect width="100%" height="100%" fill="#fff"/>
<path d="{1}" fill="#000"/>
</svg>"##,
        view_size, path
    ))
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
    );

    response
        .headers_mut()
        .insert(PRAGMA, HeaderValue::from_static("no-cache"));

    response
}

fn error_response(status: StatusCode, message: &str) -> Response {
    no_store(
        (
            status,
            Html(layout(
                "Ошибка",
                &format!(
                    r#"
<h1>Ошибка</h1>
<div class="alert error">{}</div>
"#,
                    escape_html(message)
                ),
            )),
        )
            .into_response(),
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
