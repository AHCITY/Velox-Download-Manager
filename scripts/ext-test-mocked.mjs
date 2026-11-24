#!/usr/bin/env node
// Mocked-browser-API regression tests for the extension service worker
// (download-engine/extension/background.js). Runs the REAL script in a node:vm
// sandbox with a chrome-API mock — no browser needed. Complements the
// real-Chrome E2E (ext-test-sw-eval.mjs), which is the actual-browser verdict.
//
// Covers (spec §1.6): context-menu creation without duplicate errors,
// request ids + concurrent requests, timeout handling, disconnect/reconnect,
// host unavailable, interception consent, self-download exclusion.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SW = readFileSync(path.join(ROOT, 'download-engine/extension/background.js'), 'utf8');

function makeChrome() {
  const state = {
    nativeConnects: 0,
    ports: [],
    menus: [],
    menuCreateErrors: [],
    removedAll: 0,
    notifications: [],
    badgeTexts: [],
    storage: {},
    sent: [],
    installed: null,
  };
  const chrome = {
    runtime: {
      id: 'ffnpjmbhfmaamhlbohbmamnmoaikpdmc',
      lastError: null,
      getManifest: () => ({ version: '1.0.0' }),
      connectNative: (name) => {
        assert.equal(name, 'io.velox.downloadmanager');
        state.nativeConnects++;
        const port = {
          name,
          onMessage: { listeners: [], addListener(f) { this.listeners.push(f); } },
          onDisconnect: { listeners: [], addListener(f) { this.listeners.push(f); } },
          postMessage: (m) => { state.lastSent = m; state.sent.push(m); },
          disconnect: () => {},
        };
        state.ports.push(port);
        return port;
      },
      onInstalled: { listeners: [], addListener(f) { this.listeners.push(f); } },
      onMessage: { listeners: [], addListener(f) { this.listeners.push(f); } },
      sendMessage: () => {},
    },
    storage: {
      local: {
        // MV3 promise style AND legacy callback style (getPrefs uses the promise)
        get(defaults, cb) {
          const merged = Promise.resolve({ ...defaults, ...state.storage });
          return cb ? (merged.then(cb), undefined) : merged;
        },
        set(obj, cb) {
          const done = Promise.resolve(Object.assign(state.storage, obj));
          return cb ? (done.then(cb), undefined) : done;
        },
      },
    },
    contextMenus: {
      removeAll: (cb) => { state.menus.length = 0; state.removedAll++; cb?.(); },
      create: (opts, cb) => {
        if (state.menus.some((m) => m.id === opts.id)) {
          chrome.runtime.lastError = { message: `Cannot create item with duplicate id ${opts.id}` };
          state.menuCreateErrors.push(opts.id);
        } else {
          state.menus.push(opts);
        }
        cb?.();
      },
      onClicked: { listeners: [], addListener(f) { this.listeners.push(f); } },
    },
    alarms: { create: () => {}, onAlarm: { listeners: [], addListener(f) { this.listeners.push(f); } } },
    notifications: {
      create: (idOrOpts, maybeOpts) => {
        const id = typeof idOrOpts === 'string' ? idOrOpts : `n${state.notifications.length}`;
        state.notifications.push({ id, opts: typeof idOrOpts === 'string' ? maybeOpts : idOrOpts });
      },
      clear: (id, cb) => cb?.(),
      onButtonClicked: { listeners: [], addListener(f) { this.listeners.push(f); }, removeListener(f) { const i = this.listeners.indexOf(f); if (i >= 0) this.listeners.splice(i, 1); } },
    },
    downloads: { onCreated: { listeners: [], addListener(f) { this.listeners.push(f); } } },
    action: {
      setBadgeText: (o) => state.badgeTexts.push(o.text),
      setBadgeBackgroundColor: () => {},
    },
    permissions: { request: async () => true },
  };
  return { chrome, state };
}

const loadSW = (c) => loadSWInner(c);
function loadSWInner({ chrome, state }) {
  const ctx = {
    chrome,
    navigator: { userAgent: 'Mozilla/5.0 Chrome/120.0' },
    console,
    setTimeout,
    clearTimeout,
    URL,
    Promise,
    VELOX_TEST_REQUEST_TIMEOUT_MS: 80,
    VELOX_TEST_RECONNECT_MS: 10,
  };
  vm.createContext(ctx);
  vm.runInContext(SW, ctx, { filename: 'background.js' });
  return ctx;
}

const tick = (ms = 20) => new Promise((r) => setTimeout(r, ms));

const reply = (port, env) => port.onMessage.listeners.forEach((f) => f(env));

test('install creates the three context menus after removeAll (no duplicate errors)', () => {
  const c = makeChrome();
  loadSW(c);
  c.chrome.runtime.onInstalled.listeners.forEach((f) => f({ reason: 'install' }));
  assert.equal(c.state.removedAll, 1, 'removeAll before create');
  assert.deepEqual(c.state.menus.map((m) => m.id),
    ['velox-download-link', 'velox-download-page', 'velox-download-selection']);
  assert.equal(c.state.menuCreateErrors.length, 0);
  // extension update / service-worker reload fires onInstalled again — the
  // pre-fix script threw "Cannot create item with duplicate id" here
  c.chrome.runtime.onInstalled.listeners.forEach((f) => f({ reason: 'update' }));
  assert.equal(c.state.removedAll, 2);
  assert.equal(c.state.menus.length, 3, 'menu set recreated cleanly');
  assert.equal(c.state.menuCreateErrors.length, 0, 'no duplicate-id errors');
});

test('hello handshake: connect ack sets connected + app version in the badge', async () => {
  const c = makeChrome();
  loadSW(c);
  await tick();
  const port = c.state.ports[0];
  const hello = c.state.lastSent;
  assert.equal(hello.type, 'hello');
  assert.equal(hello.version, 1);
  assert.ok(hello.id, 'request id present');
  reply(port, { version: 1, id: hello.id, type: 'hello_ack', payload: { connected: true, app_version: '0.6.0' } });
  await tick();
  assert.equal(c.state.badgeTexts.at(-1), '', 'connected → no ! badge');
});

test('host unavailable: hello_ack connected=false keeps the ! badge and error text', async () => {
  const c = makeChrome();
  loadSW(c);
  await tick();
  const port = c.state.ports[0];
  reply(port, { version: 1, id: c.state.lastSent.id, type: 'hello_ack', payload: { connected: false, reason: 'Velox is not running' } });
  await tick();
  assert.equal(c.state.badgeTexts.at(-1), '!');
});

test('request timeout resolves with E_TIMEOUT instead of hanging', async () => {
  const c = makeChrome();
  const ctx = loadSW(c);
  await new Promise((r) => setTimeout(r, 30));
  const t0 = Date.now();
  const res = await ctx.request('status_request', {});
  const dt = Date.now() - t0;
  assert.equal(res.type, 'error');
  assert.equal(res.payload.code, 'E_TIMEOUT');
  assert.ok(dt < 3000, `timeout honored in ${dt}ms`);
});

test('two concurrent requests are matched by id, not by arrival order', async () => {
  const c = makeChrome();
  const ctx = loadSW(c);
  await tick();
  const port = c.state.ports[0];
  const p1 = ctx.request('status_request', {});
  await tick();
  const p2 = ctx.request('add_request', { url: 'https://example.com/a.zip' });
  await tick();
  const [, m1, m2] = c.state.sent; // [0]=hello, [1]=status_request, [2]=add_request
  // answer in REVERSE order — id matching must untangle it
  reply(port, { version: 1, id: m2.id, type: 'download_accepted', payload: { download_id: 42 } });
  reply(port, { version: 1, id: m1.id, type: 'status_response', payload: { active: 3, agg_bps: 0, recent: [] } });
  const r1 = await p1;
  const r2 = await p2;
  assert.equal(r1.type, 'status_response', 'p1 got the status reply');
  assert.equal(r2.type, 'download_accepted', 'p2 got its own reply');
  assert.equal(r1.payload.active, 3);
  assert.equal(r2.payload.download_id, 42);
});

test('disconnect schedules a reconnect and connectNative is retried', async () => {
  const c = makeChrome();
  loadSW(c);
  await new Promise((r) => setTimeout(r, 30));
  const before = c.state.nativeConnects;
  c.state.ports[0].onDisconnect.listeners.forEach((f) => f());
  await new Promise((r) => setTimeout(r, 60));
  assert.ok(c.state.nativeConnects > before, 'reconnected after disconnect');
});

test('auto interception submits a big http download; self-downloads are excluded', async () => {
  const c = makeChrome();
  const ctx = loadSW(c);
  await new Promise((r) => setTimeout(r, 30));
  await vm.runInContext(`chrome.storage.local.set({ interceptMode: 'auto' })`, ctx);
  c.chrome.downloads.onCreated.listeners.forEach((f) => f({
    id: 1, url: 'https://example.com/big.bin', totalBytes: 3 * 1024 * 1024, filename: 'C:\\Downloads\\big.bin',
  }));
  await new Promise((r) => setTimeout(r, 30));
  const sent = c.state.lastSent;
  assert.equal(sent.type, 'add_request');
  assert.equal(sent.payload.url, 'https://example.com/big.bin');
  // own downloads (byExtensionId) must never be intercepted
  c.chrome.downloads.onCreated.listeners.forEach((f) => f({
    id: 2, url: 'https://example.com/x.zip', totalBytes: 5 * 1024 * 1024,
    byExtensionId: 'ffnpjmbhfmaamhlbohbmamnmoaikpdmc',
  }));
  await new Promise((r) => setTimeout(r, 30));
  assert.equal(c.state.lastSent.payload.url, 'https://example.com/big.bin', 'no add_request for own download');
});

test('small and non-http downloads never trigger interception', async () => {
  const c = makeChrome();
  const ctx = loadSW(c);
  await new Promise((r) => setTimeout(r, 30));
  await vm.runInContext(`chrome.storage.local.set({ interceptMode: 'auto' })`, ctx);
  c.chrome.downloads.onCreated.listeners.forEach((f) => f({
    id: 3, url: 'https://example.com/tiny.bin', totalBytes: 1024,
  }));
  c.chrome.downloads.onCreated.listeners.forEach((f) => f({
    id: 4, url: 'chrome-extension://abcdef/gone.bin', totalBytes: 99999999,
  }));
  await new Promise((r) => setTimeout(r, 30));
  assert.equal(c.state.lastSent.type, 'hello', 'only the initial hello was ever sent');
});

console.log('ext-test-moked: all assertions executed in-process');
