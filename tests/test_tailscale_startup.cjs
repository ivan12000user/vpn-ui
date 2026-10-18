// Headless Tailscale page startup: catch browser JavaScript exceptions before
// they disable refresh, filtering or the two Ping buttons on Pixel.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const input = fs.readFileSync('src/web/mod.rs', 'utf8');
const match = input.match(/const TAILSCALE_SCRIPT: &str = r#"([\s\S]*?)"#;/);
assert.ok(match, 'Tailscale JavaScript missing');

function eventTarget() {
  const events = new Map();
  return {events,addEventListener(type, handler) {events.set(type,handler);}};
}
function makeButton(result) {
  return {
    disabled:false,
    parentElement:{querySelector(css) {
      assert.equal(css,'.ping-result');
      return result;
    }}
  };
}
function pingResult() {
  return {textContent:'',classList:{toggle(){}}};
}
const resultTS = pingResult(), resultICMP = pingResult();
const buttonTS = makeButton(resultTS), buttonICMP = makeButton(resultICMP);
const row = {
  dataset:{tsIp:'100.88.88.8'},
  querySelector(css) {
    return {'.ts-overlay-button':buttonTS,'.ts-ping-button':buttonICMP}[css];
  }
};
const tbody = Object.assign(eventTarget(),{rows:[row]});
const refresh = eventTarget(),filter = Object.assign(eventTarget(),{value:''});
const pingAll = eventTarget(), auto = {checked:true};
const elements = {
  'tailscale-rows':tbody,'ts-refresh':refresh,'ts-ping-all':pingAll,
  'ts-auto':auto,'ts-updated':{},'ts-filter':filter,'ts-error':{}
};
let interval=0;
vm.runInNewContext(match[1],{
  document:{getElementById(id){return elements[id]||null;},hidden:false},
  setInterval(_fn,ms){assert.equal(ms,15000);interval++;},
  fetch(){throw Error('Unexpected network request during initialization');},
  console,Map,Set,Array,Date,Number,Promise
});
assert.equal(resultTS.textContent,'—');
assert.equal(resultICMP.textContent,'—');
assert.equal(buttonTS.disabled,false);
assert.equal(buttonICMP.disabled,false);
assert.ok(refresh.events.has('click'),'refresh listener not attached');
assert.ok(filter.events.has('input'),'filter listener not attached');
assert.ok(tbody.events.has('click'),'ping click listener not attached');
assert.equal(interval,1);
console.log('TAILSCALE startup: native Ping, ICMP, refresh, filter, timer PASS');
