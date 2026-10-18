//! Persistent read-only historical measurements collected by vpn-ui-history.timer.
use axum::{
    Json, Router,
    extract::Query,
    http::header::CACHE_CONTROL,
    response::{Html, IntoResponse},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::process::Command;

#[derive(Deserialize, Default)]
struct HistoryQuery {
    hours: Option<u16>,
    provider: Option<String>,
    peer: Option<String>,
}

pub fn router() -> Router {
    Router::new()
        .route("/history", get(page))
        .route("/api/history", get(api))
}

async fn api(Query(query): Query<HistoryQuery>) -> impl IntoResponse {
    let hours = match query.hours {
        Some(168) => 168,
        _ => 24,
    };
    let provider = query.provider.unwrap_or_default();
    let peer = query.peer.unwrap_or_default();
    let allowed = ["", "wireguard", "amneziawg", "amneziawg31", "tailscale"];
    if !allowed.contains(&provider.as_str()) || peer.len() > 128 {
        return (
            [(CACHE_CONTROL, "no-store")],
            Json(json!({"configured":false,"hours":hours,"peers":[],"points":[],
                "error":"invalid history request"})),
        );
    }
    let command = std::env::var("VPN_UI_HISTORY_COMMAND")
        .unwrap_or_else(|_| "/usr/local/libexec/vpn-ui-history".into());
    let mut child = Command::new(command);
    child.args(["query", &hours.to_string(), &provider, &peer]).kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(8), child.output()).await;
    let data = match result {
        Ok(Ok(out)) if out.status.success() && out.stdout.len() <= 1024 * 1024 => {
            serde_json::from_slice::<Value>(&out.stdout).unwrap_or_else(|_| empty(hours))
        }
        _ => empty(hours),
    };
    ([(CACHE_CONTROL, "no-store")], Json(data))
}

fn empty(hours: u16) -> Value {
    json!({"configured":false,"hours":hours,"peers":[],"points":[]})
}

async fn page() -> Html<String> {
    let body = format!(
        r#"
<div class="page-head">
  <div>
    <h1>История соединений</h1>
    <p class="lead">Сохранённые проверки VPN и Tailscale. Интервал — 5 минут, хранение — 14 дней.</p>
  </div>
  <div class="toolbar">
    <select id="hist-hours" class="filter-input" aria-label="Период">
      <option value="24">Последние 24 часа</option>
      <option value="168">Последние 7 дней</option>
    </select>
    <button id="hist-refresh" class="button" type="button">↻ Обновить</button>
  </div>
</div>
<div class="panel">
  <p class="muted" id="hist-msg" role="status">Загрузка истории...</p>
  <div class="summary" id="hist-summary" hidden>
    <div><span>Устройство</span><strong id="hist-selected">—</strong></div>
    <div><span>Измерений Ping</span><strong id="hist-probes">—</strong></div>
    <div><span>Успешных ответов</span><strong id="hist-uptime">—</strong></div>
    <div><span>Последний Ping</span><strong id="hist-last">—</strong></div>
  </div>
  <div class="toolbar" style="margin:12px 0">
    <label for="hist-provider">Протокол</label>
    <select id="hist-provider" class="filter-input" aria-label="Протокол">
      <option value="">Все</option>
      <option value="wireguard">WireGuard</option>
      <option value="amneziawg">AmneziaWG 2.0</option>
      <option value="amneziawg31">AmneziaWG 3.1</option>
      <option value="tailscale">Tailscale</option>
    </select>
    <label for="hist-peer">Устройство</label>
    <select id="hist-peer" class="filter-input" aria-label="Устройство"></select>
  </div>
  <p class="muted">Линия — задержка успешных Ping; красные точки — отсутствие ответа. Пропуск означает отсутствие измерения, а не аварию.</p>
  <div id="hist-graph" style="overflow:hidden;min-height:190px" aria-label="График задержки Ping"></div>
  <p class="muted" id="hist-detail"></p>
  <p class="muted">Для WG/AWG статус «активен» определяется свежестью handshake (до 3 минут), а не непрерывной доступностью. Процент успешных Ping считается только среди проведённых проверок.</p>
</div>
<script>{script}</script>
"#,
        script = SCRIPT,
    );
    Html(crate::web::layout("История", &body))
}

const SCRIPT: &str = r#"
(() => {
  const $ = id => document.getElementById(id);
  const range = $('hist-hours');
  const provider = $('hist-provider');
  const peer = $('hist-peer');
  const message = $('hist-msg');
  const graph = $('hist-graph');
  const details = $('hist-detail');
  const summary = $('hist-summary');
  const namespace = 'http://www.w3.org/2000/svg';
  const protocol = {
    tailscale: 'Tailscale', wireguard: 'WireGuard',
    amneziawg: 'AmneziaWG 2.0', amneziawg31: 'AmneziaWG 3.1'
  };
  let peers = [];
  let busy = false;

  const node = (name, attrs = {}, content = '') => {
    const n = document.createElementNS(namespace, name);
    for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, String(v));
    if (content) n.textContent = content;
    return n;
  };
  const fmt = ts => new Date(ts * 1000).toLocaleString('ru-RU',
    {day:'2-digit',month:'2-digit',hour:'2-digit',minute:'2-digit'});
  const label = p => (protocol[p.provider] || p.provider) + ' · ' +
    (p.label || p.ip || p.peer_id.slice(0, 10));

  function fillDevices() {
    const old = peer.value;
    peer.replaceChildren();
    const filtered = peers.filter(p => !provider.value || p.provider === provider.value);
    for (const item of filtered) {
      const option = document.createElement('option');
      option.value = item.provider + ':' + item.peer_id;
      option.textContent = label(item);
      peer.appendChild(option);
    }
    if (filtered.some(p => p.provider + ':' + p.peer_id === old)) peer.value = old;
    return filtered.find(p => p.provider + ':' + p.peer_id === peer.value) || null;
  }

  function draw(points, hours) {
    graph.replaceChildren();
    const width = 900, height = 240, left = 48, right = 15, top = 15, bottom = 33;
    const base = height - bottom, plotW = width - left - right;
    const chart = node('svg',{viewBox:'0 0 900 240',
      style:'width:100%;height:auto;display:block',
      role:'img','aria-label':'История задержки в миллисекундах'});
    const now = Math.floor(Date.now() / 1000);
    const from = now - hours * 3600;
    const good = points.filter(p => p.ping_ok === 1 && Number.isFinite(p.rtt_ms));
    const max = Math.max(30, ...good.map(p => p.rtt_ms));
    const maxY = Math.ceil(max * 1.15 / 10) * 10;
    const x = ts => left + Math.max(0,Math.min(1,(ts-from)/(hours*3600)))*plotW;
    const y = ms => top + (maxY-ms)/maxY * (base-top);
    for (const v of [0,0.5,1]) {
      const yy = y(v*maxY);
      chart.appendChild(node('line',{x1:left,y1:yy,x2:width-right,y2:yy,
        stroke:'currentColor','stroke-opacity':'.14'}));
      chart.appendChild(node('text',{x:5,y:yy+4,fill:'currentColor',
        'font-size':12},(v*maxY).toFixed(0)));
    }
    for (let k = 0; k < 5; k++) {
      const t = from + (hours*3600*k/4);
      chart.appendChild(node('text',{x:x(t),y:height-8,
        'text-anchor':k===0?'start':k===4?'end':'middle',
        fill:'currentColor','font-size':12},fmt(t)));
    }
    let last = null;
    let path = '';
    for (const p of points) {
      if (p.ping_ok === 1 && Number.isFinite(p.rtt_ms)) {
        const px = x(p.ts), py = y(p.rtt_ms);
        path += (last && p.ts-last.ts <= 660 ? 'L':'M') + px.toFixed(1) +
          ',' + py.toFixed(1) + ' ';
        last = p;
      } else {
        if (p.ping_ok === 0) {
          chart.appendChild(node('circle',{cx:x(p.ts),cy:base-3,r:3.2,fill:'#e05656'}));
        }
        last = null;
      }
    }
    chart.appendChild(node('path',{d:path,fill:'none',stroke:'#16a34a',
      'stroke-width':2.5,'stroke-linejoin':'round','stroke-linecap':'round'}));
    graph.appendChild(chart);
  }

  async function load() {
    if (busy) return;
    busy = true;
    $('hist-refresh').disabled = true;
    try {
      const hours = Number(range.value);
      const response = await fetch('/api/history?hours=' + hours,{cache:'no-store'});
      if (!response.ok) throw Error('HTTP ' + response.status);
      const data = await response.json();
      peers = data.peers || [];
      message.textContent = data.configured
        ? (peers.length ? '' : 'История пока пуста. Первые измерения появятся после запуска таймера.')
        : 'Сбор истории ещё не включён на сервере.';
      if (data.error) message.textContent = 'Ошибка истории: ' + data.error;
      const chosen = fillDevices();
      await loadSeries(chosen);
    } catch (_) {
      message.textContent = 'Не удалось загрузить историю';
      summary.hidden = true;
    } finally {
      $('hist-refresh').disabled = false;
      busy = false;
    }
  }
  async function loadSeries(chosen) {
    graph.replaceChildren();
    summary.hidden = !chosen;
    if (!chosen) { details.textContent = ''; return; }
    $('hist-selected').textContent = label(chosen);
    $('hist-probes').textContent = String(chosen.probes) + ' / ' + String(chosen.samples);
    $('hist-uptime').textContent = chosen.probes
      ? (100 * chosen.replies / chosen.probes).toFixed(1) + '%'
      : 'Нет измерений';
    $('hist-last').textContent = fmt(chosen.last_sample);
    const search = new URLSearchParams({
      hours:range.value,provider:chosen.provider,peer:chosen.peer_id
    });
    try {
      const resp = await fetch('/api/history?' + search,{cache:'no-store'});
      if (!resp.ok) throw Error();
      const data = await resp.json();
      const points = data.points || [];
      draw(points,Number(range.value));
      const latest = [...points].reverse().find(p => p.ping_ok === 1);
      details.textContent = latest
        ? 'Последний успешный ответ: ' + fmt(latest.ts) +
          ' · ' + latest.rtt_ms.toFixed(1) + ' мс' +
          (latest.path ? ' · ' + latest.path + (latest.relay ? ' ' + latest.relay : '') : '')
        : 'Успешных Ping за выбранный период пока нет.';
    } catch (_) { details.textContent = 'Ошибка чтения графика'; }
  }

  $('hist-refresh').addEventListener('click',load);
  range.addEventListener('change',load);
  provider.addEventListener('change',() => loadSeries(fillDevices()));
  peer.addEventListener('change',() => {
    const chosen = peers.find(p => p.provider + ':' + p.peer_id === peer.value);
    loadSeries(chosen || null);
  });
  load();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_has_history_controls_and_safe_chart() {
        assert!(SCRIPT.contains("/api/history?hours="));
        assert!(SCRIPT.contains("createElementNS"));
        assert!(!SCRIPT.contains(".innerHTML"));
    }

    #[test]
    fn error_response_has_no_fake_measurements() {
        let d = empty(168);
        assert_eq!(d["configured"], false);
        assert_eq!(d["points"].as_array().unwrap().len(), 0);
    }
}
