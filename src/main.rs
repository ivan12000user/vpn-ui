use axum::{
    Json, Router,
    response::{Html, IntoResponse},
    routing::get,
};
use serde::Serialize;
use std::{env, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    application: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        application: "vpn-ui",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn dashboard() -> impl IntoResponse {
    Html(page(
        "Обзор",
        r#"
        <h1>VPN UI</h1>
        <p class="lead">Единая панель управления WireGuard и AmneziaWG.</p>

        <div class="cards">
          <a class="card" href="/wireguard">
            <h2>WireGuard</h2>
            <div class="value">wg0</div>
            <div class="muted">UDP 51820</div>
          </a>

          <a class="card" href="/amneziawg">
            <h2>AmneziaWG</h2>
            <div class="value">awg0</div>
            <div class="muted">UDP 8443</div>
          </a>
        </div>

        <p class="muted">
          Версия 0.1.0 — read-only foundation.
        </p>
        "#,
    ))
}

async fn wireguard() -> impl IntoResponse {
    Html(page(
        "WireGuard",
        r#"
        <h1>WireGuard</h1>
        <p class="lead">Интерфейс wg0</p>

        <div class="panel">
          <h2>Пиры</h2>
          <p>Следующий этап: чтение активных пиров wg0.</p>
        </div>

        <div class="panel">
          <h2>Настройки</h2>
          <p>Следующий этап: безопасное чтение настроек WireGuard.</p>
        </div>
        "#,
    ))
}

async fn amneziawg() -> impl IntoResponse {
    Html(page(
        "AmneziaWG",
        r#"
        <h1>AmneziaWG</h1>
        <p class="lead">Интерфейс awg0</p>

        <div class="panel">
          <h2>Пиры</h2>
          <p>Следующий этап: чтение активных пиров awg0.</p>
        </div>

        <div class="panel">
          <h2>Настройки</h2>
          <p>Следующий этап: безопасное чтение настроек AmneziaWG.</p>
        </div>
        "#,
    ))
}

fn page(title: &str, content: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} — VPN UI</title>
<style>
:root {{
  color-scheme: light dark;
  --bg: #111418;
  --panel: #1a1f26;
  --border: #303740;
  --text: #edf1f5;
  --muted: #98a3ad;
  --accent: #68a8ff;
}}

* {{ box-sizing: border-box; }}

body {{
  margin: 0;
  font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: var(--bg);
  color: var(--text);
}}

.layout {{
  min-height: 100vh;
  display: grid;
  grid-template-columns: 230px 1fr;
}}

nav {{
  border-right: 1px solid var(--border);
  padding: 24px 16px;
}}

.brand {{
  font-size: 20px;
  font-weight: 700;
  margin: 0 8px 28px;
}}

nav a {{
  display: block;
  color: var(--text);
  text-decoration: none;
  padding: 11px 12px;
  border-radius: 8px;
  margin-bottom: 5px;
}}

nav a:hover {{
  background: var(--panel);
}}

main {{
  padding: 38px;
  max-width: 1200px;
}}

h1 {{
  margin-top: 0;
  font-size: 32px;
}}

.lead {{
  color: var(--muted);
  margin-bottom: 30px;
}}

.cards {{
  display: grid;
  grid-template-columns: repeat(auto-fit,minmax(260px,1fr));
  gap: 18px;
  margin-bottom: 30px;
}}

.card, .panel {{
  border: 1px solid var(--border);
  background: var(--panel);
  border-radius: 12px;
  padding: 22px;
}}

.card {{
  text-decoration: none;
  color: var(--text);
}}

.card:hover {{
  border-color: var(--accent);
}}

.card h2, .panel h2 {{
  margin-top: 0;
}}

.value {{
  font-size: 26px;
  font-weight: 700;
  margin: 15px 0 4px;
}}

.muted {{
  color: var(--muted);
}}

.panel {{
  margin-bottom: 18px;
}}

@media (max-width: 720px) {{
  .layout {{
    grid-template-columns: 1fr;
  }}
  nav {{
    border-right: 0;
    border-bottom: 1px solid var(--border);
  }}
  main {{
    padding: 22px;
  }}
}}
</style>
</head>
<body>
<div class="layout">
<nav>
  <div class="brand">VPN UI</div>
  <a href="/">Обзор</a>
  <a href="/wireguard">WireGuard</a>
  <a href="/amneziawg">AmneziaWG</a>
</nav>
<main>
{content}
</main>
</div>
</body>
</html>"#
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vpn_ui=info,tower_http=info".into()),
        )
        .init();

    let app = Router::new()
        .route("/", get(dashboard))
        .route("/wireguard", get(wireguard))
        .route("/amneziawg", get(amneziawg))
        .route("/healthz", get(health))
        .layer(TraceLayer::new_for_http());

    let listen = env::var("VPN_UI_LISTEN").unwrap_or_else(|_| "127.0.0.1:8090".to_string());

    let addr: SocketAddr = listen.parse().expect("invalid VPN_UI_LISTEN");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");

    info!("vpn-ui {} listening on {}", env!("CARGO_PKG_VERSION"), addr);

    axum::serve(listener, app).await.expect("server failed");
}
