// Offline headless regression test for the SVG history graph.
// Uses Node built-ins only; performs no network requests.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const source = fs.readFileSync('src/history.rs', 'utf8');
const match = source.match(/const SCRIPT: &str = r#"([\s\S]*?)"#;/);
assert.ok(match, 'history client script not found');

class FakeNode {
  constructor(tag) {
    this.tagName = tag;
    this.attributes = {};
    this.children = [];
    this.value = '';
    this.hidden = false;
    this.textContent = '';
    this.dataset = {};
  }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  appendChild(child) {
    if (this.tagName === 'hist-peer' && this.children.length === 0) {
      this.value = child.value;
    }
    this.children.push(child);
    return child;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = [...children]; }
  addEventListener() {}
}

async function testScenario(points, expectedMarkers, lineContains) {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, new FakeNode(id));
    return elements.get(id);
  };
  element('hist-hours').value = '24';
  element('hist-provider').value = '';
  element('hist-peer').value = '';
  const now = Math.floor(Date.now() / 1000);
  const selected = {
    provider:'tailscale',peer_id:'100.125.53.1',label:'garage-pc',
    ip:'100.125.53.1',samples:4,probes:3,replies:2,last_sample:now
  };
  const context = {
    document:{
      getElementById: element,
      createElement: name=>new FakeNode(name),
      createElementNS: (_ns,name)=>new FakeNode(name),
    },
    Date, Number, URLSearchParams, console,
    fetch: async url => ({
      ok:true,
      json: async () => String(url).includes('provider=')
        ? {configured:true,points}
        : {configured:true,peers:[selected]}
    }),
  };
  vm.runInNewContext(match[1], context);
  // Let both chained async fetch responses and loadSeries run to completion.
  for (let i=0;i<12;i++) await new Promise(setImmediate);
  const chart = element('hist-graph').children.find(n=>n.tagName==='svg');
  assert.ok(chart,'chart SVG must exist');
  const dots = chart.children.filter(n=>n.tagName==='circle');
  assert.equal(dots.length, expectedMarkers);
  const path = chart.children.find(n=>n.tagName==='path');
  assert.ok(path);
  if (lineContains) assert.ok(path.attributes.d.includes(lineContains), path.attributes.d);
  assert.match(element('hist-uptime').textContent, /2\/3 \(66\.7%\)/);
  return {dots,path};
}

(async()=>{
  const now=Math.floor(Date.now()/1000);
  // One successful Ping + one explicitly unprobed slot MUST produce a
  // visible green marker, not only an invisible single-point SVG path.
  const one=await testScenario([
    {ts:now-700, ping_ok:1, rtt_ms:144.7, path:'DERP', relay:'ams'},
    {ts:now-400, ping_ok:null,rtt_ms:null,path:'',relay:''}
  ],1);
  assert.equal(one.dots[0].attributes.fill, '#16a34a');

  // The skipped sample is unknown, so two short-range successes can be joined.
  const two=await testScenario([
    {ts:now-1200,ping_ok:1,rtt_ms:145},
    {ts:now-900,ping_ok:null,rtt_ms:null},
    {ts:now-300,ping_ok:1,rtt_ms:142}
  ],2,'L');

  // A real failure must have a visible RED point and break the line.
  const failed=await testScenario([
    {ts:now-1200,ping_ok:1,rtt_ms:145},
    {ts:now-900,ping_ok:0,rtt_ms:null},
    {ts:now-300,ping_ok:1,rtt_ms:142}
  ],3);
  assert.equal(failed.dots.filter(d=>d.attributes.fill==='#e05656').length,1);
  assert.equal((failed.path.attributes.d.match(/M/g)||[]).length,2);
  console.log('HISTORY SVG: single Ping, unknown gap and explicit failure PASS');
})().catch(error=>{console.error(error);process.exitCode=1});
