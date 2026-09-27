mod admin;
mod geoip;
mod vpn;
mod web;

use crate::geoip::GeoIpService;
use crate::vpn::common::ProviderConfig;
use crate::web::AppState;
use std::{env, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vpn_ui=info,tower_http=info".into()),
        )
        .init();

    let geoip = GeoIpService::new(
        env::var("VPN_UI_GEOIP_CACHE")
            .unwrap_or_else(|_| "/var/lib/vpn-ui/geoip-cache.json".to_string()),
        7 * 24 * 60 * 60,
    )
    .await;

    let state = AppState {
        wireguard: ProviderConfig {
            label: "WireGuard".to_string(),
            command: env::var("VPN_UI_WG_COMMAND").unwrap_or_else(|_| "wg".to_string()),
            metadata_command: env::var("VPN_UI_WG_METADATA_COMMAND")
                .unwrap_or_else(|_| "vpn-ui-wg-meta".to_string()),
            interface: env::var("VPN_UI_WG_INTERFACE").unwrap_or_else(|_| "wg0".to_string()),
        },

        amneziawg: ProviderConfig {
            label: "AmneziaWG".to_string(),
            command: env::var("VPN_UI_AWG_COMMAND").unwrap_or_else(|_| "awg".to_string()),
            metadata_command: env::var("VPN_UI_AWG_METADATA_COMMAND")
                .unwrap_or_else(|_| "vpn-ui-awg-meta".to_string()),
            interface: env::var("VPN_UI_AWG_INTERFACE").unwrap_or_else(|_| "awg0".to_string()),
        },

        ping_command: env::var("VPN_UI_PING_COMMAND")
            .unwrap_or_else(|_| "/usr/bin/ping".to_string()),

        wg_settings_command: env::var("VPN_UI_WG_SETTINGS_COMMAND")
            .unwrap_or_else(|_| "/usr/local/bin/vpn-ui-wg-settings".to_string()),

        awg_settings_command: env::var("VPN_UI_AWG_SETTINGS_COMMAND")
            .unwrap_or_else(|_| "/usr/local/bin/vpn-ui-awg-settings".to_string()),

        wg_client_config_command: env::var("VPN_UI_WG_CLIENT_CONFIG_COMMAND")
            .unwrap_or_else(|_| "/usr/local/bin/vpn-ui-wg-client-config".to_string()),

        awg_client_config_command: env::var("VPN_UI_AWG_CLIENT_CONFIG_COMMAND")
            .unwrap_or_else(|_| "/usr/local/bin/vpn-ui-awg-client-config".to_string()),

        geoip,
    };

    let app = web::router(state).layer(TraceLayer::new_for_http());

    let listen = env::var("VPN_UI_LISTEN").unwrap_or_else(|_| "127.0.0.1:8090".to_string());

    let addr: SocketAddr = listen.parse().expect("invalid VPN_UI_LISTEN");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");

    info!("vpn-ui {} listening on {}", env!("CARGO_PKG_VERSION"), addr);

    axum::serve(listener, app).await.expect("server failed");
}
