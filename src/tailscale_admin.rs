//! Read-only Tailscale admin API snapshot for users and *open* device invites.
//! Never return inviteUrl (bearer credential) or Tailscale API tokens to the browser.
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use serde_json::Value;
use std::{
    env, fs,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const CACHE_TTL: Duration = Duration::from_secs(180);
const MAX_DEVICES: usize = 64;
const API_BASE: &str = "https://api.tailscale.com/api/v2";

#[derive(Clone, Debug, Default, Serialize)]
pub struct ShareDirectory {
    pub configured: bool,
    pub users: Vec<DirectoryUser>,
    pub invites: Vec<DeviceInvite>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DirectoryUser {
    pub login_name: String,
    pub display_name: String,
    pub kind: String,
    pub status: String,
    pub currently_connected: Option<bool>,
    pub last_seen: String,
    pub device_count: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceInvite {
    pub device: String,
    pub email: String,
    pub allow_exit_node: Option<bool>,
    pub created: String,
    pub multi_use: bool,
}

static CACHE: OnceLock<Mutex<Option<(Instant, ShareDirectory)>>> = OnceLock::new();
static OAUTH_CACHE: OnceLock<Mutex<Option<(Instant, String)>>> = OnceLock::new();

fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_' | b'@'))
}

fn field(v: &Value, name: &str) -> String {
    v.get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn list<'a>(v: &'a Value, name: &str) -> Result<&'a [Value], &'static str> {
    match v {
        Value::Null => Ok(&[]),
        Value::Array(items) => Ok(items),
        Value::Object(map) => map
            .get(name)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .ok_or("missing array in API response"),
        _ => Err("unexpected API response"),
    }
}

fn read_users(doc: &Value) -> Result<Vec<DirectoryUser>, &'static str> {
    let mut users = Vec::new();
    for u in list(doc, "users")? {
        users.push(DirectoryUser {
            login_name: field(u, "loginName"),
            display_name: field(u, "displayName"),
            kind: field(u, "type"),
            status: field(u, "status"),
            currently_connected: u.get("currentlyConnected").and_then(Value::as_bool),
            last_seen: normalized_time(&field(u, "lastSeen")),
            device_count: u.get("deviceCount").and_then(Value::as_u64),
        });
    }
    users.sort_by(|a, b| {
        a.login_name
            .to_lowercase()
            .cmp(&b.login_name.to_lowercase())
    });
    Ok(users)
}

fn normalized_time(time: &str) -> String {
    if time.starts_with("0001-01-01") {
        String::new()
    } else {
        time.to_owned()
    }
}

fn value_id(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|x| x.to_string()))
        .unwrap_or_default()
}

fn read_devices(doc: &Value) -> Result<Vec<(String, String)>, &'static str> {
    let mut devices = Vec::new();
    for device in list(doc, "devices")? {
        let id = value_id(&device["id"]);
        if !safe_id(&id) {
            continue;
        }
        let name = ["hostname", "name"]
            .iter()
            .map(|f| field(device, f))
            .find(|s| !s.is_empty())
            .unwrap_or_else(|| id.clone());
        devices.push((id, name));
    }
    Ok(devices)
}

fn read_invites(doc: &Value, device: &str) -> Result<Vec<DeviceInvite>, &'static str> {
    // Tailscale returns a bare array for device invites. Some older clients
    // wrap it in "deviceInvites"; support both without exposing unknown fields.
    let values = match doc {
        Value::Object(map) => {
            if map.contains_key("deviceInvites") {
                list(doc, "deviceInvites")?
            } else {
                list(doc, "invites")?
            }
        }
        _ => list(doc, "deviceInvites")?,
    };
    Ok(values
        .iter()
        .map(|invite| DeviceInvite {
            device: device.to_owned(),
            email: field(invite, "email"),
            allow_exit_node: invite.get("allowExitNode").and_then(Value::as_bool),
            created: field(invite, "created"),
            multi_use: invite
                .get("multiUse")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect())
}

async fn get_json(client: &reqwest::Client, token: &str, route: &str) -> Result<Value, String> {
    // URL host is hard-coded. Never accept a user-controlled API base URL.
    let response = client
        .get(format!("{API_BASE}/{route}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "connection or TLS failure".to_owned())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response
        .json::<Value>()
        .await
        .map_err(|_| "invalid JSON response".to_owned())
}

async fn collect(tailnet: &str, token: &str) -> ShareDirectory {
    let mut output = ShareDirectory {
        configured: true,
        ..Default::default()
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            output
                .warnings
                .push("Cannot initialize HTTPS client".into());
            return output;
        }
    };

    let users_route = format!("tailnet/{tailnet}/users");
    let devices_route = format!("tailnet/{tailnet}/devices");
    let (users, devices) = tokio::join!(
        get_json(&client, token, &users_route),
        get_json(&client, token, &devices_route),
    );

    match users {
        Ok(doc) => match read_users(&doc) {
            Ok(items) => output.users = items,
            Err(e) => output.warnings.push(format!("Users: {e}")),
        },
        Err(e) => output.warnings.push(format!("Users: {e}")),
    }

    let mut device_list = match devices {
        Ok(doc) => match read_devices(&doc) {
            Ok(items) => items,
            Err(e) => {
                output.warnings.push(format!("Devices: {e}"));
                Vec::new()
            }
        },
        Err(e) => {
            output.warnings.push(format!("Devices: {e}"));
            Vec::new()
        }
    };

    if device_list.len() > MAX_DEVICES {
        device_list.truncate(MAX_DEVICES);
        output.warnings.push(format!(
            "Only first {MAX_DEVICES} devices checked; invite inventory incomplete"
        ));
    }

    let tasks = stream::iter(device_list.into_iter().map(|(id, name)| {
        let c = client.clone();
        let key = token.to_owned();
        async move {
            let result = get_json(&c, &key, &format!("device/{id}/device-invites")).await;
            (name, result)
        }
    }))
    .buffer_unordered(4);

    tokio::pin!(tasks);
    while let Some((name, result)) = tasks.next().await {
        match result {
            Ok(doc) => match read_invites(&doc, &name) {
                Ok(invites) => output.invites.extend(invites),
                Err(e) => output.warnings.push(format!("Invites for {name}: {e}")),
            },
            Err(e) => output.warnings.push(format!("Invites for {name}: {e}")),
        }
    }
    output.invites.sort_by(|a, b| {
        a.device
            .to_lowercase()
            .cmp(&b.device.to_lowercase())
            .then_with(|| a.email.to_lowercase().cmp(&b.email.to_lowercase()))
    });
    output
}


// OAuth tokens expire in one hour. Cache for 45 minutes and request a fresh
// token as needed. Credentials never enter environment variables or logs.
async fn oauth_bearer(client_id: &str, secret_file: &str) -> Result<String, String> {
    let mutex = OAUTH_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().await;
    if let Some((when, value)) = guard.as_ref() {
        if when.elapsed() < Duration::from_secs(45 * 60) {
            return Ok(value.clone());
        }
    }

    let secret = fs::read_to_string(secret_file)
        .map_err(|_| "OAuth credential cannot be read".to_owned())?;
    let secret = secret.trim();
    if secret.is_empty() || secret.len() > 2048 {
        return Err("OAuth credential invalid".to_owned());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "OAuth client initialization failed".to_owned())?;
    let response = client
        .post(format!("{API_BASE}/oauth/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", secret),
        ])
        .send()
        .await
        .map_err(|_| "OAuth token connection failed".to_owned())?;
    if !response.status().is_success() {
        return Err(format!("OAuth token HTTP {}", response.status().as_u16()));
    }
    let doc = response
        .json::<Value>()
        .await
        .map_err(|_| "OAuth token JSON invalid".to_owned())?;
    let access_token = field(&doc, "access_token");
    if access_token.is_empty() {
        return Err("OAuth token missing".to_owned());
    }
    *guard = Some((Instant::now(), access_token.clone()));
    Ok(access_token)
}

async fn load_bearer() -> Result<Option<String>, String> {
    if let (Ok(id), Ok(path)) = (
        env::var("VPN_UI_TS_OAUTH_CLIENT_ID"),
        env::var("VPN_UI_TS_OAUTH_SECRET_FILE"),
    ) {
        if id.trim().is_empty() || path.trim().is_empty() {
            return Ok(None);
        }
        return oauth_bearer(&id, &path).await.map(Some);
    }
    let Ok(path) = env::var("VPN_UI_TS_API_TOKEN_FILE") else {
        return Ok(None);
    };
    if path.trim().is_empty() {
        return Ok(None);
    }
    match fs::read_to_string(path) {
        Ok(key) if !key.trim().is_empty() && key.trim().len() <= 2048 => {
            Ok(Some(key.trim().to_owned()))
        }
        _ => Err("API credential missing or unreadable".to_owned()),
    }
}

pub async fn status() -> ShareDirectory {
    let Ok(tailnet) = env::var("VPN_UI_TS_TAILNET") else {
        return ShareDirectory::default();
    };
    if !safe_id(&tailnet) {
        return ShareDirectory {
            warnings: vec!["Invalid VPN_UI_TS_TAILNET".into()],
            ..Default::default()
        };
    }
    let is_oauth_configured = env::var("VPN_UI_TS_OAUTH_CLIENT_ID").is_ok()
        && env::var("VPN_UI_TS_OAUTH_SECRET_FILE").is_ok();
    let is_api_token_configured = env::var("VPN_UI_TS_API_TOKEN_FILE").is_ok();
    if !is_oauth_configured && !is_api_token_configured {
        return ShareDirectory::default();
    }

    let mutex = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().await;
    if let Some((when, cached)) = guard.as_ref() {
        if when.elapsed() < CACHE_TTL {
            return cached.clone();
        }
    }

    let result = match load_bearer().await {
        Ok(Some(token)) => collect(&tailnet, &token).await,
        Ok(None) => ShareDirectory {
            configured: true,
            warnings: vec!["API credential not configured".into()],
            ..Default::default()
        },
        Err(message) => ShareDirectory {
            configured: true,
            warnings: vec![message],
            ..Default::default()
        },
    };
    *guard = Some((Instant::now(), result.clone()));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn captures_open_invite_without_bearer_link() {
        let doc = json!([{
            "email": "first@example.test",
            "allowExitNode": true,
            "multiUse": false,
            "inviteUrl": "https://login.tailscale.com/admin/invite/SECRET"
        }]);
        let invites = read_invites(&doc, "VPS").unwrap();
        assert!(invites[0].allow_exit_node.unwrap());
        let serialized = serde_json::to_string(&invites).unwrap();
        assert!(!serialized.contains("SECRET"));
        assert!(!serialized.contains("inviteUrl"));
    }

    #[test]
    fn users_do_not_invent_online_state() {
        let users = read_users(&json!({"users":[
            {"loginName":"alice@example.test","type":"shared"},
            {"loginName":"bob@example.test","currentlyConnected":false}
        ]}))
        .unwrap();
        assert_eq!(users[0].currently_connected, None);
        assert_eq!(users[1].currently_connected, Some(false));
    }

    #[test]
    fn supports_empty_and_wrapped_invites() {
        assert_eq!(read_invites(&Value::Null, "VPS").unwrap().len(), 0);
        assert_eq!(
            read_invites(&json!({"deviceInvites":[]}), "VPS")
                .unwrap()
                .len(),
            0
        );
        assert!(read_invites(&json!({"unexpected":[]}), "VPS").is_err());
    }

    #[test]
    fn rejects_untrusted_path_components() {
        assert!(!safe_id("a/b"));
        assert!(!safe_id("bad?query"));
        assert!(safe_id("example.ts.net"));
        assert!(safe_id("owner@example.com"));
    }
}

pub const PANEL: &str = r#"
<section class="panel" id="ts-directory">
  <h2>Пользователи и приглашения Tailscale</h2>
  <p class="muted">Данные административного API Tailscale, только просмотр. Онлайн ≠ использование Exit Node.
  API приглашений содержит только открытые приглашения; принятые доступы проверяются отдельно в админ-панели Tailscale.
  Доступ участников tailnet определяется ACL/grants и не выводится из списка приглашений.</p>
  <div class="toolbar"><button type="button" class="button" id="ts-directory-refresh">↻ Проверить данные</button>
  <span class="muted" id="ts-directory-status">Загрузка…</span></div>
  <h3>Пользователи tailnet и внешние пользователи</h3>
  <div class="table-wrap"><table><thead><tr><th>Аккаунт</th><th>Тип</th><th>Статус Tailscale</th><th>Последняя активность</th><th>Устройства</th><th>Доступ к конкретным узлам</th></tr></thead>
  <tbody id="ts-directory-users"><tr><td colspan="6">Загрузка…</td></tr></tbody></table></div>
  <h3>Открытые приглашения к отдельным узлам</h3>
  <div class="table-wrap"><table><thead><tr><th>Узел</th><th>Кому направлено</th><th>Статус</th><th>Exit Node разрешён</th><th>Создано</th></tr></thead>
  <tbody id="ts-directory-invites"><tr><td colspan="5">Загрузка…</td></tr></tbody></table></div>
  <p class="muted">Показаны только открытые приглашения. Уже принятые разрешения тут не перечисляются. Ссылки приглашений скрыты. Если API вернул неполные сведения, появится предупреждение; отсутствие строки не означает отсутствие доступа.</p>
  <p class="muted">API не предоставляет здесь точную связь «внешний пользователь → все уже расшаренные ему узлы». Для такой матрицы потребуется дополнительная проверка источника.</p>
</section>
<script>
(() => {
  const status = document.getElementById('ts-directory-status');
  const usersBody = document.getElementById('ts-directory-users');
  const invitesBody = document.getElementById('ts-directory-invites');
  const button = document.getElementById('ts-directory-refresh');
  let busy = false;
  function cell(row, value) {
    const el = document.createElement('td');
    el.textContent = value === null || value === undefined || value === '' ? '—' : String(value);
    row.appendChild(el);
  }
  function clear(node) { node.replaceChildren(); }
  function empty(node, cols, message) {
    const row = document.createElement('tr');
    const el = document.createElement('td');
    el.colSpan = cols;
    el.textContent = message;
    row.appendChild(el);
    node.appendChild(row);
  }
  function when(raw) {
    if (!raw || String(raw).startsWith('0001-')) return '—';
    const date = new Date(raw);
    return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString('ru-RU');
  }
  function connection(value) {
    return value === true ? 'ONLINE' : value === false ? 'OFFLINE' : 'Неизвестно';
  }
  function refreshResult(data) {
    clear(usersBody);
    clear(invitesBody);
    if (!data.configured) {
      status.textContent = 'Административный API ещё не настроен на сервере';
      empty(usersBody, 6, 'Нужны настройки OAuth или файла API-токена');
      empty(invitesBody, 5, 'Нет данных');
      return;
    }
    const users = Array.isArray(data.users) ? data.users : [];
    const invites = Array.isArray(data.invites) ? data.invites : [];
    const warnings = Array.isArray(data.warnings) ? data.warnings : [];
    for (const user of users) {
      const row = document.createElement('tr');
      cell(row, user.display_name && user.login_name ? user.display_name + ' (' + user.login_name + ')' : user.login_name || user.display_name);
      cell(row, user.kind === 'shared' ? 'Внешний (shared)' : user.kind === 'member' ? 'Участник tailnet' : user.kind);
      cell(row, connection(user.currently_connected) + (user.status ? ' · ' + user.status : ''));
      cell(row, when(user.last_seen));
      cell(row, user.device_count);
      cell(row, user.kind === 'shared' ? 'Требует проверки по устройствам в Admin Console' : 'Определяется ACL/grants');
      usersBody.appendChild(row);
    }
    if (!users.length) empty(usersBody, 6, 'Пользователи не получены');
    for (const invite of invites) {
      const row = document.createElement('tr');
      cell(row, invite.device);
      cell(row, invite.email || 'Ссылка без адресата');
      cell(row, 'Ожидает');
      cell(row, invite.allow_exit_node === true ? 'Да' : invite.allow_exit_node === false ? 'Нет' : 'Неизвестно');
      cell(row, when(invite.created));
      invitesBody.appendChild(row);
    }
    if (!invites.length) empty(invitesBody, 5, 'Открытых приглашений нет или они недоступны; принятые доступы тут не отображаются');
    status.textContent = users.length + ' пользователей · ' + invites.length + ' открытых приглашений'
      + (warnings.length ? ' · Неполные данные: ' + warnings.slice(0, 3).join('; ') : '')
      + ' · серверный кеш до 3 минут';
  }
  async function refresh() {
    if (busy) return;
    busy = true;
    button.disabled = true;
    try {
      const response = await fetch('/api/tailscale/shares', {cache: 'no-store', credentials: 'same-origin'});
      if (!response.ok) throw new Error('HTTP ' + response.status);
      refreshResult(await response.json());
    } catch (_) {
      status.textContent = 'Ошибка получения сведений Tailscale API';
    } finally {
      busy = false;
      button.disabled = false;
    }
  }
  button.addEventListener('click', refresh);
  refresh();
  setInterval(refresh, 60000);
})();
</script>
"#;
