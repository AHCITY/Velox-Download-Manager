// Velox extension service worker — connection management, context menu,
// browser-download interception, popup data source.
//
// Design: the ONLY durable state is chrome.storage.local preferences; download
// state lives in the Velox desktop app (single source of truth, queried on
// demand). The native-messaging port reconnects with capped backoff; every
// outbound message carries a request id and protocol version.

const HOST_NAME = 'io.velox.downloadmanager';
const PROTOCOL_VERSION = 1;
// test hooks: the mocked-SW suite (scripts/ext-test-mocked.mjs) shortens these
// so timeout/reconnect paths run in milliseconds instead of wall-clock minutes
const REQUEST_TIMEOUT_MS = globalThis.VELOX_TEST_REQUEST_TIMEOUT_MS || 20000;
const RECONNECT_BASE_MS = globalThis.VELOX_TEST_RECONNECT_MS || 1000;
const MAX_POLL_MS = 15000;
const INTERCEPT_MIN_BYTES = 1 * 1024 * 1024;

let port = null;
let connected = false;
let appVersion = null;
let lastError = null;
let reconnectTimer = null;
let reconnectDelay = RECONNECT_BASE_MS;
let seq = 0;
const pending = new Map(); // request id -> {resolve}
let lastStatus = { active: 0, aggBps: 0, recent: [] };

/* ---------- prefs (the only persistent state) ---------- */
async function getPrefs() {
  return chrome.storage.local.get({ interceptMode: 'ask', clipboardEnabled: false });
}

/* ---------- native messaging ---------- */
function sendHello() {
  request('hello', { ext_version: chrome.runtime.getManifest().version, browser: browserLabel() })
    .then((ack) => {
      connected = !!(ack && ack.type === 'hello_ack' && ack.payload && ack.payload.connected === true);
      appVersion = ack && ack.payload ? ack.payload.app_version || null : null;
      if (ack && ack.payload && ack.payload.reason) lastError = ack.payload.reason;
      if (connected) { lastError = null; reconnectDelay = RECONNECT_BASE_MS; }
      refreshBadge();
    })
    .catch(() => {});
}

function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST_NAME);
  } catch (e) {
    connected = false;
    // connectNative itself only throws when the API is absent (the manifest
    // lacks "nativeMessaging"); registration problems surface via onDisconnect
    lastError = 'native messaging unavailable — the manifest needs the nativeMessaging permission and the host must be registered';
    scheduleReconnect();
    return;
  }
  port.onMessage.addListener((msg) => {
    const entry = pending.get(String(msg.id));
    if (entry) {
      pending.delete(String(msg.id));
      entry.resolve(msg);
    }
    if (msg.type === 'status_response') {
      lastStatus = {
        active: msg.payload.active || 0,
        aggBps: msg.payload.agg_bps || 0,
        recent: msg.payload.recent || [],
        appVersion: msg.payload.app_version || null,
      };
      refreshBadge();
    }
  });
  port.onDisconnect.addListener(() => {
    const err = chrome.runtime.lastError;
    if (err) lastError = err.message || 'host disconnected';
    port = null;
    connected = false;
    refreshBadge();
    scheduleReconnect();
  });
  sendHello();
}

function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    reconnectDelay = Math.min(reconnectDelay * 2, MAX_POLL_MS);
    connect();
  }, reconnectDelay);
}
function request(type, payload = {}) {
  return new Promise((resolve) => {
    if (!port) connect();
    if (!port) {
      resolve({ type: 'error', payload: { code: 'E_NO_HOST', message: 'native host unreachable' } });
      return;
    }
    const id = String(++seq);
    pending.set(id, { resolve });
    try {
      port.postMessage({ version: PROTOCOL_VERSION, id, type, payload });
    } catch (e) {
      pending.delete(id);
      resolve({ type: 'error', payload: { code: 'E_PORT', message: String(e) } });
      return;
    }
    setTimeout(() => {
      if (pending.has(id)) {
        pending.delete(id);
        resolve({ type: 'error', payload: { code: 'E_TIMEOUT', message: 'no response from Velox' } });
      }
    }, REQUEST_TIMEOUT_MS);
  });
}

/* ---------- helpers ---------- */
function browserLabel() {
  const ua = navigator.userAgent || '';
  if (ua.includes('Edg/')) return 'edge';
  if (ua.includes('OPR/')) return 'opera';
  return 'chromium';
}

function fmtBytes(n) {
  if (n == null) return '—';
  if (n < 1024) return n + ' B';
  const u = ['KiB', 'MiB', 'GiB', 'TiB'];
  let v = n, i = -1;
  do { v /= 1024; i++; } while (v >= 1024 && i < u.length - 1);
  return (v >= 100 ? v.toFixed(0) : v.toFixed(1)) + ' ' + u[i];
}

function notify(title, message) {
  try {
    chrome.notifications.create({
      type: 'basic',
      iconUrl: 'icons/128.png',
      title: title || 'Velox',
      message: String(message || '').slice(0, 300),
    });
  } catch (e) { /* notifications unavailable in some contexts */ }
}

async function refreshBadge() {
  try {
    if (connected && lastStatus.active > 0) {
      await chrome.action.setBadgeText({ text: String(lastStatus.active) });
      await chrome.action.setBadgeBackgroundColor({ color: '#4d9fff' });
    } else if (connected) {
      await chrome.action.setBadgeText({ text: '' });
    } else {
      await chrome.action.setBadgeText({ text: '!' });
      await chrome.action.setBadgeBackgroundColor({ color: '#f4695e' });
    }
  } catch (e) { /* action API race during startup */ }
}

async function refreshStatus() {
  const r = await request('status_request', {});
  if (r.type === 'status_response') connected = true;
  return lastStatus;
}

/* ---------- URL submission ---------- */
async function submitUrl(url, source, tab) {
  const r = await request('add_request', {
    url,
    source: source || 'extension',
    page_url: tab && tab.url ? tab.url : undefined,
    referrer: tab && tab.url && /^https?:\/\//i.test(tab.url) ? tab.url : undefined,
  });
  if (r.type === 'download_accepted') {
    notify('Velox', (r.payload.filename ? r.payload.filename + ' — ' : '') + 'download added to Velox');
  } else if (r.type === 'download_rejected') {
    notify('Velox', 'Velox rejected the URL: ' + (r.payload.message || 'unknown'));
  } else {
    notify('Velox', (r.payload && r.payload.message) || 'Velox is unreachable — start the desktop app');
  }
  refreshStatus();
  return r;
}

/* ---------- context menu ----------
 * removeAll() before create: on extension update/reload Chrome keeps the old
 * menus only until the SW unloads, but a duplicate create() throws
 * "Cannot create item with duplicate id" and aborts the remaining creates.
 * Recreating idempotently makes every install/reload/update path identical. */
chrome.runtime.onInstalled.addListener(async () => {
  chrome.contextMenus.removeAll(() => {
    const create = (id, title, contexts) =>
      chrome.contextMenus.create({ id, title, contexts }, () => void chrome.runtime.lastError);
    create('velox-download-link', 'Download with Velox', ['link']);
    create('velox-download-page', 'Download page URL with Velox', ['page', 'frame']);
    create('velox-download-selection', 'Download selected URL with Velox', ['selection']);
  });
  connect();
});

// MV3 keeps non-persistent listeners but unloads idle workers; a light alarm
// re-asserts the connection (and is the only background timer we use).
chrome.alarms.create('velox-keepalive', { periodInMinutes: 30 });
chrome.alarms.onAlarm.addListener((a) => { if (a.name === 'velox-keepalive') connect(); });

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl
    || (/^https?:\/\//i.test((info.selectionText || '').trim()) ? info.selectionText.trim() : null)
    || info.pageUrl
    || info.frameUrl;
  if (!url || !/^https?:\/\//i.test(url)) {
    notify('Velox', 'No http(s) URL found in that context');
    return;
  }
  await submitUrl(url, 'context_menu', tab);
});

/* ---------- browser download interception (consent-first) ---------- */
chrome.downloads.onCreated.addListener(async (item) => {
  const prefs = await getPrefs();
  if (prefs.interceptMode === 'off') return;
  // only offer for substantial binary resources; never hijack trivial things
  if (item.totalBytes && item.totalBytes < INTERCEPT_MIN_BYTES) return;
  if (!item.url || !/^https?:\/\//i.test(item.url)) return;
  if (item.byExtensionId) return; // never react to our own downloads

  if (prefs.interceptMode === 'ask') {
    const nid = 'velox-offer-' + item.id;
    chrome.notifications.create(nid, {
      type: 'basic',
      iconUrl: 'icons/128.png',
      title: 'Send this download to Velox?',
      message: (item.filename ? item.filename.split(/[\\/]/).pop() : 'This download')
        + (item.totalBytes ? ' (' + fmtBytes(item.totalBytes) + ')' : ''),
      buttons: [{ title: 'Download with Velox' }, { title: 'Keep browser download' }],
      requireInteraction: true,
    });
    const handler = (notifId, buttonIndex) => {
      if (notifId !== nid) return;
      chrome.notifications.onButtonClicked.removeListener(handler);
      chrome.notifications.clear(nid);
      if (buttonIndex === 0) {
        try { chrome.downloads.cancel(item.id, () => void chrome.runtime.lastError); } catch (e) { /* already gone */ }
        submitUrl(item.url, 'intercept_ask', null);
      }
    };
    chrome.notifications.onButtonClicked.addListener(handler);
    setTimeout(() => {
      chrome.notifications.onButtonClicked.removeListener(handler);
      chrome.notifications.clear(nid);
    }, 30000);
  } else if (prefs.interceptMode === 'auto') {
    submitUrl(item.url, 'intercept_auto', null);
  }
});

/* ---------- popup bridge ---------- */
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  (async () => {
    if (msg.type === 'get_state') {
      const status = await refreshStatus();
      const prefs = await getPrefs();
      sendResponse({ connected, appVersion, error: lastError, active: status.active, aggBps: status.aggBps, recent: status.recent, prefs });
    } else if (msg.type === 'add_url') {
      const r = await submitUrl(msg.url, 'popup', null);
      sendResponse({ ok: r.type === 'download_accepted', detail: r.payload && r.payload.message });
    } else if (msg.type === 'reconnect') {
      if (port) { try { port.disconnect(); } catch (e) { /* already gone */ } port = null; }
      reconnectDelay = RECONNECT_BASE_MS;
      connect();
      sendResponse({ ok: true });
    } else if (msg.type === 'set_pref') {
      await chrome.storage.local.set({ [msg.key]: msg.value });
      if (msg.key === 'clipboardEnabled' && msg.value) {
        const granted = await chrome.permissions.request({ permissions: ['clipboardRead'] });
        if (!granted) await chrome.storage.local.set({ clipboardEnabled: false });
        sendResponse({ ok: granted });
      } else {
        sendResponse({ ok: true });
      }
    }
  })();
  return true; // async response
});

connect();
