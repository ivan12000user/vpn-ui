//! Local read-only inventory. Never expose raw CLI JSON or authentication data.
use serde::{Deserialize, Serialize};
use std::{
    process::Stdio,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command, sync::Mutex};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Node {
    address: Option<String>,
    version: Option<String>,
    online: Option<bool>,
    tcp_fallback_active: Option<bool>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Network {
    id: Option<String>,
    name: Option<String>,
    status: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    port_device_name: Option<String>,
    assigned_addresses: Vec<String>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Peer {
    address: Option<String>,
    role: Option<String>,
    latency: Option<i64>,
    version: Option<String>,
}
#[derive(Clone, Default, Serialize)]
pub struct Status {
    node: Option<Node>,
    networks: Option<Vec<Network>>,
    peers: Option<Vec<Peer>>,
    errors: Vec<String>,
}

const LIMIT: u64 = 1024 * 1024;
const READS: [&str; 3] = ["info", "listnetworks", "listpeers"];

async fn read(cli: &str, operation: &str) -> Result<Vec<u8>, String> {
    if !READS.contains(&operation) {
        return Err("Unsupported read".into());
    }
    let mut child = Command::new(cli)
        .args(["-j", operation])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "CLI unavailable or permission denied")?;
    let mut output = Vec::new();
    let mut stdout = child.stdout.take().ok_or("Missing stdout")?.take(LIMIT + 1);
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(|_| "Cannot read CLI output")?;
        if output.len() as u64 > LIMIT {
            return Err("CLI output too large");
        }
        if !child
            .wait()
            .await
            .map_err(|_| "Cannot wait for CLI")?
            .success()
        {
            return Err("CLI failed or permission denied");
        }
        Ok(())
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(output),
        other => {
            let _ = child.kill().await;
            Err(match other {
                Err(_) => "CLI timed out",
                Ok(Err(e)) => e,
                _ => unreachable!(),
            }
            .into())
        }
    }
}

async fn collect<T: for<'de> Deserialize<'de>>(
    cli: &str,
    op: &str,
    errors: &mut Vec<String>,
) -> Option<T> {
    let result = match read(cli, op).await {
        Ok(bytes) => parse(&bytes, op).map_err(String::from),
        Err(e) => Err(e),
    };
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            errors.push(format!("{op}: {e}"));
            None
        }
    }
}

fn parse<T: for<'de> Deserialize<'de>>(bytes: &[u8], op: &str) -> Result<T, &'static str> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid JSON schema")?;
    if (op == "info" && !value.is_object()) || (op != "info" && !value.is_array()) {
        return Err("Invalid JSON schema");
    }
    serde_json::from_value(value).map_err(|_| "Invalid JSON schema")
}

pub async fn status() -> Status {
    static CACHE: OnceLock<Mutex<Option<(Instant, Status)>>> = OnceLock::new();
    // Single flight, including failures, prevents browser refreshes spawning a process storm.
    let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().await;
    if let Some((at, value)) = &*cache {
        if at.elapsed() < Duration::from_secs(10) {
            return value.clone();
        }
    }
    let cli = std::env::var("VPN_UI_ZEROTIER_COMMAND")
        .unwrap_or_else(|_| "/usr/local/bin/vpn-ui-zerotier-ro".into());
    let mut value = Status::default();
    value.node = collect(&cli, "info", &mut value.errors).await;
    value.networks = collect(&cli, "listnetworks", &mut value.errors).await;
    value.peers = collect(&cli, "listpeers", &mut value.errors).await;
    *cache = Some((Instant::now(), value.clone()));
    value
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn text(value: &Option<String>) -> String {
    esc(value.as_deref().unwrap_or("—"))
}
pub fn dashboard_card() -> String {
    "<a class=\"card\" href=\"/zerotier\"><h2>ZeroTier</h2><p>Локальный мониторинг · только чтение →</p></a>".into()
}
pub fn page(status: &Status) -> String {
    let mut html = String::from(
        r#"<h1>ZeroTier</h1><p>Только чтение локального состояния. Обновление — при открытии страницы, кэш 10 секунд.</p>
<p><a class="button zt-refresh" href="/zerotier">↻ Обновить</a></p>
<style>.zt-refresh{display:inline-flex;align-items:center;min-height:44px}.zt-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,300px),1fr));gap:12px}.zt-card{min-width:0;overflow-wrap:anywhere}.zt-card dl{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:8px}.zt-card dd{margin:0;min-width:0}.zt-card dt{color:#94a3b8}</style>"#,
    );
    for error in &status.errors {
        html.push_str(&format!(
            "<p role=\"status\">Недоступно: {}</p>",
            esc(error)
        ));
    }
    if let Some(node) = &status.node {
        html.push_str(&format!("<section class=\"panel zt-card\"><h2>Локальный узел</h2><dl><dt>Адрес</dt><dd>{}</dd><dt>Версия</dt><dd>{}</dd><dt>Онлайн</dt><dd>{}</dd><dt>TCP fallback</dt><dd>{}</dd></dl></section>", text(&node.address), text(&node.version), flag(node.online), flag(node.tcp_fallback_active)));
    }
    html.push_str("<h2>Подключённые сети</h2><div class=\"zt-grid\">");
    if let Some(networks) = &status.networks {
        if networks.is_empty() {
            html.push_str("<p>Нет подключённых сетей.</p>");
        }
        for n in networks {
            html.push_str(&format!("<section class=\"panel zt-card\"><h3>{}</h3><dl><dt>ID</dt><dd>{}</dd><dt>Состояние</dt><dd>{}</dd><dt>Тип</dt><dd>{}</dd><dt>Интерфейс</dt><dd>{}</dd><dt>Адреса</dt><dd>{}</dd></dl></section>", text(&n.name), text(&n.id), text(&n.status), text(&n.kind), text(&n.port_device_name), esc(&n.assigned_addresses.join(", "))));
        }
    }
    html.push_str("</div><h2>Известные узлы</h2><p>Узлы, известные этому серверу, включая служебные PLANET/MOON. Это не полный список участников сети и не подтверждение доступности. Задержка — последнее известное измерение; «—» означает отсутствие данных. История и активные проверки ZeroTier не собираются.</p><div class=\"zt-grid\">");
    if let Some(peers) = &status.peers {
        if peers.is_empty() {
            html.push_str("<p>Нет известных узлов.</p>");
        }
        for p in peers {
            html.push_str(&format!("<section class=\"panel zt-card\"><h3>{}</h3><dl><dt>Роль</dt><dd>{}</dd><dt>Версия</dt><dd>{}</dd><dt>Последняя latency, мс</dt><dd>{}</dd></dl></section>", text(&p.address), text(&p.role), text(&p.version), p.latency.filter(|v| *v >= 0).map(|v| v.to_string()).unwrap_or_else(|| "—".into())));
        }
    }
    html.push_str("</div>");
    html
}
fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Да",
        Some(false) => "Нет",
        None => "Неизвестно",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn cli_failures_are_bounded_and_do_not_leak_stderr() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("vpn-ui-zt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cli");
        let cli = path.to_str().unwrap();
        for (body, expected) in [
            ("echo SECRET >&2; exit 1", "CLI failed or permission denied"),
            ("exec sleep 10", "CLI timed out"),
            ("exec head -c 1048577 /dev/zero", "CLI output too large"),
        ] {
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(read(cli, "info").await.unwrap_err(), expected);
        }
        std::fs::write(&path, "#!/bin/sh\necho not-json\n").unwrap();
        let mut errors = Vec::new();
        assert!(collect::<Node>(cli, "info", &mut errors).await.is_none());
        assert_eq!(errors, ["info: Invalid JSON schema"]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(read(cli, "info").await.is_err());
    }
    #[test]
    fn whitelist_drops_secrets_and_preserves_unknown_state() {
        let node: Node = serde_json::from_str(r#"{"address":"abc","secret":"TOKEN"}"#).unwrap();
        assert_eq!(node.online, None);
        assert!(!serde_json::to_string(&node).unwrap().contains("TOKEN"));
        assert!(serde_json::from_str::<Vec<Network>>("{}").is_err());
        assert!(parse::<Node>(b"[]", "info").is_err());
    }
    #[test]
    fn renders_untrusted_names_and_partial_failure_safely() {
        let status = Status {
            networks: Some(vec![Network {
                name: Some("<script>\"&".into()),
                ..Default::default()
            }]),
            errors: vec!["listpeers: unavailable".into()],
            ..Default::default()
        };
        let html = page(&status);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;&quot;&amp;"));
        assert!(html.contains("listpeers: unavailable"));
        assert!(!html.contains("Нет известных узлов"));
    }
    #[tokio::test]
    async fn mutation_is_rejected_before_execution() {
        assert!(
            read("does-not-exist", "join")
                .await
                .unwrap_err()
                .contains("Unsupported")
        );
    }
}
