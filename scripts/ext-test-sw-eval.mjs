#!/usr/bin/env node
// Real-browser extension E2E (audit EXT): drives Chrome for Testing over CDP.
// Verifies: (1) the unpacked extension loads and its service worker runs,
// (2) chrome.runtime.id equals the ID derived from the manifest key,
// (3) native messaging connects and the host answers hello (Velox reachable),
// (4) status_request returns live rows through host -> bridge -> host -> SW.
// A real browser download being caught is asserted by the orchestrator from
// the fake bridge's request log (download.create).
//
// Usage: node ext-test-sw-eval.mjs [debugPort] [fileUrl]
const DEBUG_PORT = process.argv[2] || '9333';
const FILE_URL = process.argv[3] || 'http://127.0.0.1:9344/big.bin';
const EXPECTED_ID = 'ffnpjmbhfmaamhlbohbmamnmoaikpdmc';
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function targets() {
  const r = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json`);
  return r.json();
}

const EXT_PREFIX = `chrome-extension://${EXPECTED_ID}/`;
let sw = null;
for (let i = 0; i < 50 && !sw; i++) {
  await wait(400);
  for (const t of await targets()) {
    // match OUR extension only — Chrome runs built-in component workers too
    if ((t.url || '').startsWith(EXT_PREFIX)) { sw = t; break; }
  }
}
if (!sw) {
  console.error('FAIL: no target for the Velox extension (id ' + EXPECTED_ID + ')');
  console.error(JSON.stringify(await targets(), null, 1).slice(0, 3000));
  process.exit(1);
}
console.log('OK: Velox extension target running at', sw.url, '(type', sw.type + ')');

const ws = new WebSocket(sw.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = (e) => rej(new Error('ws error')); });
let id = 0;
const pend = new Map();
ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m.result); pend.delete(m.id); } };
const send = (method, params = {}) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const evalSW = async (expr) => (await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true })).result?.value;

const runtimeId = await evalSW('chrome.runtime.id');
console.log('extension runtime id:', runtimeId, runtimeId === EXPECTED_ID ? '(matches the derived ID)' : '(MISMATCH!)');

const hello = await evalSW(`new Promise((res) => {
  try {
    const p = chrome.runtime.connectNative('io.velox.downloadmanager');
    let done = false;
    p.onMessage.addListener((m) => { if (!done) { done = true; res(m); p.disconnect(); } });
    p.onDisconnect.addListener(() => { if (!done) res({ DISCONNECTED: chrome.runtime.lastError ? chrome.runtime.lastError.message : 'pipe closed' }); });
    p.postMessage({ version: 1, id: 't1', type: 'hello', payload: { ext_version: '1.0.0', browser: 'chromium-testing' } });
    setTimeout(() => { if (!done) res({ TIMEOUT: 'no reply in 12s' }); }, 12000);
  } catch (e) { res({ THROW: String(e) }); }
})`);
console.log('hello reply:', JSON.stringify(hello));

await evalSW(`chrome.storage.local.set({ interceptMode: 'auto' })`);
const tab = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json/new?${encodeURIComponent(FILE_URL)}`, { method: 'PUT' })
  .then((r) => r.json())
  .catch((e) => ({ error: String(e) }));
console.log('opened download tab:', tab.url || JSON.stringify(tab));

let status = null;
for (let i = 0; i < 10; i++) {
  await wait(800);
  status = await evalSW(`new Promise((res) => {
    const p = chrome.runtime.connectNative('io.velox.downloadmanager');
    p.onMessage.addListener((m) => { res(m); p.disconnect(); });
    p.onDisconnect.addListener(() => res({ DISCONNECTED: chrome.runtime.lastError ? chrome.runtime.lastError.message : 'pipe closed' }));
    p.postMessage({ version: 1, id: 't2', type: 'status_request', payload: {} });
    setTimeout(() => res({ TIMEOUT: 'no status reply' }), 6000);
  })`);
  if (status && status.type === 'status_response' && status.payload && status.payload.active > 0) break;
}
console.log('status reply:', JSON.stringify(status));

let fail = 0;
if (runtimeId !== EXPECTED_ID) { console.error('FAIL: runtime id != derived ID'); fail = 1; }
if (!(hello && hello.type === 'hello_ack' && hello.payload && hello.payload.connected === true)) { console.error('FAIL: hello not connected'); fail = 1; }
if (!(status && status.type === 'status_response' && status.payload && status.payload.active > 0)) { console.error('FAIL: status active is not > 0'); fail = 1; }
console.log(fail ? 'SW-EVAL: FAILED' : 'SW-EVAL: ALL CHECKS PASSED');
process.exit(fail);
