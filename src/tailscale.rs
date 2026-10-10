//! Read-only local Tailscale peer inventory.
//! The control-plane netmap is NOT a list of people invited to use a shared
//! exit node. That separate inventory requires administrator-side data.
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

#[derive(Clone, Debug, Default, Serialize)]
pub struct TailPeer {
    pub hostname: String,
    pub ip: String,
    pub owner: String,
    pub online: bool,
    pub active: bool,
    pub exit_node_option: bool,
    pub relay: String,
    pub last_seen: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TailStatus {
    pub backend_state: String,
    pub self_hostname: String,
    pub self_ip: String,
    pub self_exit_node_option: bool,
    pub peers: Vec<TailPeer>,
    pub online_count: usize,
    pub error: Option<String>,
}

fn text_field(v: &Value, field: &str) -> String {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn bool_field(v: &Value, field: &str) -> bool {
    v.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn ipv4(v: &Value) -> String {
    v.get("TailscaleIPs")
        .and_then(Value::as_array)
        .and_then(|xs| xs.iter().filter_map(Value::as_str).find(|s| !s.contains(':')))
        .unwrap_or_default()
        .to_owned()
}

fn parse_status(input: &[u8]) -> Result<TailStatus, String> {
    let doc: Value = serde_json::from_slice(input).map_err(|e| format!("Invalid Tailscale JSON: {e}"))?;
    let self_node = doc.get("Self").unwrap_or(&Value::Null);
    let users = doc.get("User").and_then(Value::as_object);
    let mut peers = Vec::new();

    if let Some(map) = doc.get("Peer").and_then(Value::as_object) {
        for v in map.values() {
            let uid = v.get("UserID").and_then(Value::as_i64).map(|x| x.to_string()).unwrap_or_default();
            let owner = users
                .and_then(|u| u.get(&uid))
                .map(|u| text_field(u, "LoginName"))
                .unwrap_or_default();

            peers.push(TailPeer {
                hostname: text_field(v, "HostName"),
                ip: ipv4(v),
                owner,
                online: bool_field(v, "Online"),
                active: bool_field(v, "Active"),
                exit_node_option: bool_field(v, "ExitNodeOption"),
                relay: text_field(v, "Relay"),
                last_seen: text_field(v, "LastSeen"),
            });
        }
    }

    peers.sort_by(|a, b| {
        b.online.cmp(&a.online)
            .then_with(|| a.hostname.to_lowercase().cmp(&b.hostname.to_lowercase()))
    });
    let online_count = peers.iter().filter(|p| p.online).count();

    Ok(TailStatus {
        backend_state: text_field(&doc, "BackendState"),
        self_hostname: text_field(self_node, "HostName"),
        self_ip: ipv4(self_node),
        self_exit_node_option: bool_field(self_node, "ExitNodeOption"),
        peers,
        online_count,
        error: None,
    })
}

pub async fn status() -> TailStatus {
    // This call only inspects local tailscaled state; no "tailscale up/set".
    // A separate privileged read-only helper can be configured if the
    // vpn-ui service account cannot access tailscaled's Unix socket.
    let cli = std::env::var("VPN_UI_TAILSCALE_COMMAND")
        .unwrap_or_else(|_| "/usr/bin/tailscale".to_owned());
    let mut command = Command::new(&cli);
    command.args(["status", "--json"]).kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(7), command.output()).await;
    match result {
        Err(_) => TailStatus { error: Some("Tailscale status timed out".to_owned()), ..Default::default() },
        Ok(Err(err)) => TailStatus { error: Some(format!("Tailscale status unavailable: {err}")), ..Default::default() },
        Ok(Ok(out)) if !out.status.success() => {
            let message = String::from_utf8_lossy(&out.stderr);
            TailStatus {
                error: Some(format!("Tailscale status error: {}", message.chars().take(240).collect::<String>())),
                ..Default::default()
            }
        }
        Ok(Ok(out)) => match parse_status(&out.stdout) {
            Ok(status) => status,
            Err(error) => TailStatus { error: Some(error), ..Default::default() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_peer_online_and_exit_capability() {
        let input = br#"{
          "BackendState": "Running",
          "Self": {"HostName":"new-vps","TailscaleIPs":["100.80.198.69"],"ExitNodeOption":true},
          "User":{"42":{"LoginName":"owner@example.test"}},
          "Peer": {
            "node1": {"HostName":"opione","TailscaleIPs":["100.92.11.114"],"Online":true,"Active":true,"Relay":"ams","UserID":42},
            "node2": {"HostName":"surface","TailscaleIPs":["100.103.235.34"],"Online":false,"Active":false,"ExitNodeOption":true}
          }
        }"#;
        let s = parse_status(input).unwrap();
        assert_eq!(s.backend_state, "Running");
        assert_eq!(s.self_ip, "100.80.198.69");
        assert!(s.self_exit_node_option);
        assert_eq!(s.peers.len(), 2);
        assert_eq!(s.online_count, 1);
        assert_eq!(s.peers[0].hostname, "opione");
        assert_eq!(s.peers[0].owner, "owner@example.test");
        assert_eq!(s.peers[0].relay, "ams");
        assert!(s.peers[1].exit_node_option);
    }

    #[test]
    fn missing_optional_fields_do_not_invent_online_state() {
        let s = parse_status(br#"{"BackendState":"Running","Peer":{"x":{"HostName":"unknown"}}}"#).unwrap();
        assert_eq!(s.peers.len(), 1);
        assert!(!s.peers[0].online);
        assert!(!s.peers[0].active);
        assert_eq!(s.peers[0].ip, "");
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_status(b"not json").is_err());
    }
}
