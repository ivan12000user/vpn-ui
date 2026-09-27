use serde::Serialize;
use std::{
    collections::HashMap,
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;
use tracing::warn;

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub label: String,
    pub command: String,
    pub metadata_command: String,
    pub interface: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PeerStatus {
    pub public_key: String,
    pub name: Option<String>,
    pub vpn_ip: Option<String>,
    pub endpoint: Option<String>,
    pub allowed_ips: String,
    pub latest_handshake: Option<u64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub persistent_keepalive: Option<u16>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InterfaceStatus {
    pub provider: String,
    pub interface: String,
    pub public_key: Option<String>,
    pub listen_port: Option<u16>,
    pub peers: Vec<PeerStatus>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PingResult {
    pub ok: bool,
    pub ip: String,
    pub avg_ms: Option<f64>,
    pub received: usize,
    pub error: Option<String>,
}

impl InterfaceStatus {
    pub fn failed(provider: &str, interface: &str, message: impl Into<String>) -> Self {
        Self {
            provider: provider.to_string(),
            interface: interface.to_string(),
            public_key: None,
            listen_port: None,
            peers: Vec::new(),
            error: Some(message.into()),
        }
    }

    pub fn is_online(&self) -> bool {
        self.error.is_none()
    }
}

pub async fn query_provider(config: &ProviderConfig) -> InterfaceStatus {
    let output = match Command::new(&config.command)
        .args(["show", &config.interface, "dump"])
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => {
            return InterfaceStatus::failed(
                &config.label,
                &config.interface,
                format!("cannot execute {}: {err}", config.command),
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return InterfaceStatus::failed(
            &config.label,
            &config.interface,
            if stderr.is_empty() {
                format!("{} returned {}", config.command, output.status)
            } else {
                stderr
            },
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut status = match parse_dump(&config.label, &config.interface, &stdout) {
        Ok(status) => status,
        Err(err) => return InterfaceStatus::failed(&config.label, &config.interface, err),
    };

    match Command::new(&config.metadata_command).output().await {
        Ok(output) if output.status.success() => {
            let metadata = String::from_utf8_lossy(&output.stdout);
            apply_metadata(&mut status, &metadata);
        }
        Ok(output) => {
            warn!(
                "{} metadata command failed: {}",
                config.label,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(err) => {
            warn!("{} metadata command failed: {}", config.label, err);
        }
    }

    status
}

fn parse_dump(provider: &str, interface: &str, dump: &str) -> Result<InterfaceStatus, String> {
    let mut lines = dump.lines();

    let interface_line = lines
        .next()
        .ok_or_else(|| "empty dump returned by VPN command".to_string())?;

    let fields: Vec<&str> = interface_line.split('\t').collect();

    if fields.len() < 3 {
        return Err(format!(
            "unexpected interface dump format: {} fields",
            fields.len()
        ));
    }

    // fields[0] = interface private key.
    // It is deliberately ignored.
    let public_key = value(fields.get(1).copied());
    let listen_port = fields.get(2).and_then(|v| v.parse::<u16>().ok());

    let mut peers = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();

        if fields.len() < 7 {
            continue;
        }

        // fields[1] = peer preshared key.
        // It is deliberately ignored.
        let allowed_ips = fields.get(3).copied().unwrap_or_default().to_string();

        let latest_handshake = fields
            .get(4)
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0);

        peers.push(PeerStatus {
            public_key: fields.first().copied().unwrap_or_default().to_string(),
            name: None,
            vpn_ip: vpn_ip_from_allowed(&allowed_ips),
            endpoint: fields.get(2).and_then(|v| value(Some(*v))),
            allowed_ips,
            latest_handshake,
            rx_bytes: fields
                .get(5)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0),
            tx_bytes: fields
                .get(6)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0),
            persistent_keepalive: fields
                .get(7)
                .and_then(|v| v.parse::<u16>().ok())
                .filter(|v| *v > 0),
        });
    }

    Ok(InterfaceStatus {
        provider: provider.to_string(),
        interface: interface.to_string(),
        public_key,
        listen_port,
        peers,
        error: None,
    })
}

fn apply_metadata(status: &mut InterfaceStatus, metadata: &str) {
    let mut names = HashMap::new();

    for line in metadata.lines() {
        let mut fields = line.splitn(2, '\t');

        let public_key = fields.next().unwrap_or_default().trim();
        let name = fields.next().unwrap_or_default().trim();

        if !public_key.is_empty() && !name.is_empty() {
            names.insert(public_key.to_string(), name.to_string());
        }
    }

    for peer in &mut status.peers {
        if let Some(name) = names.get(&peer.public_key) {
            peer.name = Some(name.clone());
        }
    }
}

fn vpn_ip_from_allowed(allowed_ips: &str) -> Option<String> {
    let first = allowed_ips.split(',').next()?.trim();
    let ip = first.split('/').next()?.trim();

    ip.parse::<IpAddr>().ok().map(|value| value.to_string())
}

fn value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "(none)" && *v != "off")
        .map(ToString::to_string)
}

pub async fn ping_ip(command: &str, ip: &str) -> PingResult {
    if ip.parse::<IpAddr>().is_err() {
        return PingResult {
            ok: false,
            ip: ip.to_string(),
            avg_ms: None,
            received: 0,
            error: Some("invalid IP address".to_string()),
        };
    }

    let output = match Command::new(command)
        .args(["-n", "-c", "3", "-W", "1", ip])
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => {
            return PingResult {
                ok: false,
                ip: ip.to_string(),
                avg_ms: None,
                received: 0,
                error: Some(format!("cannot execute ping: {err}")),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut times = Vec::new();

    for line in stdout.lines() {
        for token in line.split_whitespace() {
            if let Some(value) = token.strip_prefix("time=")
                && let Ok(ms) = value.parse::<f64>()
            {
                times.push(ms);
            } else if token.starts_with("time<") {
                times.push(0.5);
            }
        }
    }

    let received = times.len();

    if received == 0 {
        return PingResult {
            ok: false,
            ip: ip.to_string(),
            avg_ms: None,
            received: 0,
            error: Some("timeout".to_string()),
        };
    }

    let avg_ms = times.iter().sum::<f64>() / received as f64;

    PingResult {
        ok: true,
        ip: ip.to_string(),
        avg_ms: Some(avg_ms),
        received,
        error: None,
    }
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dump_without_exposing_private_material() {
        let input = concat!(
            "PRIVATE\tSERVER_PUBLIC\t51820\toff\n",
            "PEER_PUBLIC\tPSK\t1.2.3.4:12345\t10.0.0.2/32\t100\t1024\t2048\t15\n",
        );

        let mut status = parse_dump("WireGuard", "wg0", input).unwrap();

        apply_metadata(&mut status, "PEER_PUBLIC\ttest-peer\n");

        assert_eq!(status.public_key.as_deref(), Some("SERVER_PUBLIC"));
        assert_eq!(status.listen_port, Some(51820));
        assert_eq!(status.peers.len(), 1);
        assert_eq!(status.peers[0].name.as_deref(), Some("test-peer"));
        assert_eq!(status.peers[0].vpn_ip.as_deref(), Some("10.0.0.2"));

        let json = serde_json::to_string(&status).unwrap();

        assert!(!json.contains("PRIVATE"));
        assert!(!json.contains("PSK"));
    }
}
