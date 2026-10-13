//! Tailscale control-plane ping, independent of host ICMP.
//! Executes only a fixed read-only helper, with a known, live Tailscale peer.
use crate::tailscale;
use serde::Serialize;
use std::{net::Ipv4Addr, process::Stdio, sync::OnceLock, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, sync::Semaphore};

static LIMIT: OnceLock<Semaphore> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
pub struct OverlayPingResult {
    pub ok: bool,
    pub ip: String,
    pub avg_ms: Option<f64>,
    pub received: usize,
    pub path: String,
    pub relay: String,
    pub error: Option<String>,
}

impl OverlayPingResult {
    fn failed(ip: &str, reason: &str) -> Self {
        Self {
            ok: false,
            ip: ip.to_string(),
            avg_ms: None,
            received: 0,
            path: String::new(),
            relay: String::new(),
            error: Some(reason.to_string()),
        }
    }
}

// Prevent this endpoint from becoming a general-purpose server-side ping tool.
pub fn known_peer(status: &tailscale::TailStatus, ip: &str) -> bool {
    let Ok(addr) = ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let octets = addr.octets();
    if octets[0] != 100 || !(64..=127).contains(&octets[1]) {
        return false;
    }
    status.peers.iter().any(|peer| peer.ip == ip)
}

fn parse_pong(line: &str) -> Option<(f64, String, String)> {
    if !line.starts_with("pong from ") {
        return None;
    }
    let (first, time) = line.rsplit_once(" in ")?;
    let time = time.split_whitespace().next()?.strip_suffix("ms")?;
    let value: f64 = time.parse().ok()?;
    if !value.is_finite() || !(0.0..=60000.0).contains(&value) {
        return None;
    }
    let (_, via) = first.rsplit_once(" via ")?;
    if via.starts_with("DERP(") && via.ends_with(')') {
        let region = &via[5..via.len() - 1];
        if region.is_empty()
            || region.len() > 32
            || !region
                .chars()
                .all(|x| x.is_ascii_alphanumeric() || x == '-')
        {
            return None;
        }
        return Some((value, "DERP".to_string(), region.to_string()));
    }
    if via.starts_with("peer-relay") {
        return Some((value, "RELAY".to_string(), String::new()));
    }
    if via.is_empty() || via.len() > 128 {
        return None;
    }
    Some((value, "DIRECT".to_string(), String::new()))
}

fn parse_output(ip: &str, output: &str) -> OverlayPingResult {
    let mut results = Vec::new();
    let mut path = String::new();
    let mut relay = String::new();
    for line in output.lines().take(60) {
        if let Some((ms, last_path, last_relay)) = parse_pong(line) {
            results.push(ms);
            path = last_path;
            relay = last_relay;
        }
    }
    if results.is_empty() {
        return OverlayPingResult::failed(ip, "timeout");
    }
    OverlayPingResult {
        ok: true,
        ip: ip.to_string(),
        avg_ms: Some(results.iter().sum::<f64>() / results.len() as f64),
        received: results.len(),
        path,
        relay,
        error: None,
    }
}

pub async fn ping(ip: &str, status: &tailscale::TailStatus) -> OverlayPingResult {
    if status.error.is_some() || status.backend_state != "Running" {
        return OverlayPingResult::failed(ip, "Tailscale status unavailable");
    }
    if !known_peer(status, ip) {
        return OverlayPingResult::failed("", "Tailscale peer not found");
    }

    let limiter = LIMIT.get_or_init(|| Semaphore::new(4));
    let Ok(_slot) = limiter.try_acquire() else {
        return OverlayPingResult::failed(ip, "Ping limit reached");
    };

    let helper = std::env::var("VPN_UI_TS_PING_COMMAND")
        .unwrap_or_else(|_| "/usr/local/bin/vpn-ui-tailscale-ping".to_string());
    let mut child = match Command::new(&helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return OverlayPingResult::failed(ip, "Tailscale ping helper unavailable"),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(format!("{ip}\n").as_bytes()).await.is_err() {
            return OverlayPingResult::failed(ip, "Tailscale ping helper input failed");
        }
    }
    match tokio::time::timeout(Duration::from_secs(20), child.wait_with_output()).await {
        Err(_) => OverlayPingResult::failed(ip, "timeout"),
        Ok(Err(_)) => OverlayPingResult::failed(ip, "Tailscale ping helper failed"),
        Ok(Ok(output)) => {
            let result = parse_output(ip, &String::from_utf8_lossy(&output.stdout));
            if !output.status.success() && !result.ok {
                return OverlayPingResult::failed(ip, "Tailscale ping unavailable");
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_status() -> tailscale::TailStatus {
        tailscale::TailStatus {
            backend_state: "Running".into(),
            peers: vec![tailscale::TailPeer {
                ip: "100.125.53.1".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn only_known_overlay_ip_is_allowed() {
        let status = test_status();
        assert!(known_peer(&status, "100.125.53.1"));
        for bad in [
            "127.0.0.1",
            "8.8.8.8",
            "100.125.53.2",
            "100.125.53.1;rm",
            "::1",
            "100.128.0.1",
        ] {
            assert!(!known_peer(&status, bad), "{bad}");
        }
    }

    #[test]
    fn parses_derp_and_direct_ping_examples() {
        let derp = parse_output(
            "100.125.53.1",
            "pong from garage-pc (100.125.53.1) via DERP(ams) in 148ms\npong from garage-pc (100.125.53.1) via DERP(ams) in 217ms\npong from garage-pc (100.125.53.1) via DERP(ams) in 150ms\ndirect connection not established\n",
        );
        assert!(derp.ok);
        assert_eq!(derp.received, 3);
        assert_eq!(derp.path, "DERP");
        assert_eq!(derp.relay, "ams");
        assert!((derp.avg_ms.unwrap() - 171.6666).abs() < 0.01);
        let direct = parse_output(
            "100.125.53.1",
            "pong from garage-pc (100.125.53.1) via 203.0.113.5:41641 in 5.3ms\n",
        );
        assert_eq!(direct.path, "DIRECT");
        assert!((direct.avg_ms.unwrap() - 5.3).abs() < 0.01);
    }

    #[test]
    fn missing_replies_are_not_online() {
        let r = parse_output("100.125.53.1", "direct connection not established\n");
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("timeout"));
    }
}
