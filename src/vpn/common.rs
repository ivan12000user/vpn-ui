use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub label: String,
    pub command: String,
    pub interface: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PeerStatus {
    pub public_key: String,
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

    match parse_dump(&config.label, &config.interface, &stdout) {
        Ok(status) => status,
        Err(err) => InterfaceStatus::failed(&config.label, &config.interface, err),
    }
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

    // Important:
    // fields[0] is the private key.
    // It is deliberately ignored and never leaves this parser.
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

        // fields[1] is the preshared key.
        // It is deliberately ignored.
        let latest_handshake = fields
            .get(4)
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0);

        peers.push(PeerStatus {
            public_key: fields.first().copied().unwrap_or_default().to_string(),
            endpoint: fields.get(2).and_then(|v| value(Some(*v))),
            allowed_ips: fields.get(3).copied().unwrap_or_default().to_string(),
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

fn value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "(none)" && *v != "off")
        .map(ToString::to_string)
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
    fn parses_wireguard_dump_without_exposing_private_material() {
        let input = concat!(
            "PRIVATE\tSERVER_PUBLIC\t51820\toff\n",
            "PEER_PUBLIC\tPSK\t1.2.3.4:12345\t10.0.0.2/32\t100\t1024\t2048\t15\n",
        );

        let status = parse_dump("WireGuard", "wg0", input).unwrap();

        assert_eq!(status.public_key.as_deref(), Some("SERVER_PUBLIC"));
        assert_eq!(status.listen_port, Some(51820));
        assert_eq!(status.peers.len(), 1);
        assert_eq!(status.peers[0].public_key, "PEER_PUBLIC");
        assert_eq!(status.peers[0].allowed_ips, "10.0.0.2/32");

        let json = serde_json::to_string(&status).unwrap();

        assert!(!json.contains("PRIVATE"));
        assert!(!json.contains("PSK"));
    }
}
