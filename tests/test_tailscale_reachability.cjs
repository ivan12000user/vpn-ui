// Regression: control-plane ONLINE does not imply successful reachability.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const source = fs.readFileSync('src/web/mod.rs', 'utf8');
const match = source.match(/const TAILSCALE_SCRIPT: &str = r#"([\s\S]*?)"#;/);
assert.ok(match, 'Tailscale browser JS not found');
assert.ok(source.includes('<span class="ts-reachability">Не проверен</span>'));

function classList(initial = []) {
  const values = new Set(initial);
  return {
    toggle(value, enabled) {
      if (enabled) values.add(value);
      else values.delete(value);
    },
    contains(value) { return values.has(value); }
  };
}

function row(ip) {
  const reach = {textContent:'Не проверен', classList:classList()};
  const pingTS = {textContent:'', classList:classList()};
  const pingICMP = {textContent:'', classList:classList()};
  const tsButton = {disabled:false, parentElement:{querySelector(){return pingTS;}}};
  const icmpButton = {disabled:false, parentElement:{querySelector(){return pingICMP;}}};
  return {
    dataset:{tsIp:ip}, classList:classList(['status-ok']),
    reach, pingTS, tsButton,
    querySelector(selector) {
      if (selector === '.ts-overlay-button') return tsButton;
      if (selector === '.ts-ping-button') return icmpButton;
      if (selector === '.ts-reachability') return reach;
      return null;
    }
  };
}

const noReply = row('100.88.88.1');
const reachable = row('100.88.88.14');
const helperError = row('100.88.88.27');
const events = new Map(), timers = [];
const tbody = {
  rows:[noReply,reachable,helperError],
  addEventListener(name,callback){events.set('tbody:'+name,callback);}
};
const pingAll = {
  disabled:false, textContent:'Проверить все TS',
  addEventListener(name,callback){events.set('pingAll:'+name,callback);}
};
const misc = {
  'tailscale-rows':tbody,
  'ts-ping-all':pingAll,
  'ts-auto':{checked:false},
  'ts-refresh':{disabled:false,addEventListener(){}},
  'ts-filter':{value:'',addEventListener(){}},
  'ts-updated':{textContent:''},
  'ts-error':{textContent:'',className:''},
  'ts-reachable':{textContent:''}
};
let pingCalls = 0;
const context = {
  document:{
    hidden:false,
    getElementById(id){return misc[id] || null;}
  },
  setInterval(cb,ms){timers.push({cb,ms});},
  fetch(url, options){
    assert.equal(url,'/api/tailscale/overlay-ping');
    const ip = JSON.parse(options.body).ip;
    pingCalls++;
    if (ip === '100.88.88.1') {
      return Promise.resolve({ok:true,json:async()=>({
        ok:false,error:'timeout',avg_ms:null
      })});
    }
    if (ip === '100.88.88.27') {
      return Promise.resolve({ok:true,json:async()=>({
        ok:false,error:'Tailscale ping unavailable',avg_ms:null
      })});
    }
    assert.equal(ip,'100.88.88.14');
    return Promise.resolve({ok:true,json:async()=>({
      ok:true,avg_ms:101.4,path:'DIRECT',relay:''
    })});
  },
  console,Map,Set,Array,Date,Number,Promise
};

vm.runInNewContext(match[1],context);
assert.equal(pingCalls,0,'No network request should start during JS initialization');
assert.equal(timers.length,1,'Single 15-second scheduler');
assert.equal(timers[0].ms,15000);
assert.equal(noReply.pingTS.textContent,'—');
assert.equal(reachable.pingTS.textContent,'—');

(async () => {
  await events.get('pingAll:click')();
  assert.equal(pingCalls,3);
  assert.equal(noReply.reach.textContent,'Нет ответа');
  assert.ok(noReply.classList.contains('reach-unavailable'));
  assert.equal(reachable.reach.textContent,'Доступен');
  assert.ok(!reachable.classList.contains('reach-unavailable'));
  assert.ok(reachable.classList.contains('reach-online'));
  assert.equal(helperError.reach.textContent,'Ошибка проверки');
  assert.ok(!helperError.classList.contains('reach-unavailable'));
  assert.match(misc['ts-reachable'].textContent,/Доступно 1 \/ 3 проверено/);
  assert.match(misc['ts-reachable'].textContent,/нет ответа: 1/);
  assert.equal(pingAll.disabled,false);
  console.log('TAILSCALE reachability: reported ONLINE vs TS Ping timeout PASS');
})().catch(err => {console.error(err);process.exitCode = 1;});
