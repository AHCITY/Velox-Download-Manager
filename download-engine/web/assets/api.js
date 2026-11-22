// Velox frontend service layer — the ONLY module that talks to the engine.
// One JSON-RPC 2.0 contract, two transports:
//   desktop: Tauri IPC `invoke('rpc')` (engine runs in-process)
//   browser: HTTP POST /rpc + SSE /events (velox serve-http preview)
// Screens never touch Tauri or fetch directly.

const TAURI = typeof window !== 'undefined' && !!(window.__TAURI__ && window.__TAURI__.core);

let seq = 0;
const inflight = new Map();

export const isDesktop = TAURI;

async function invokeRaw(method, params) {
  const body = JSON.stringify({
    jsonrpc: '2.0',
    id: String(++seq),
    method,
    params: params || {},
  });
  if (TAURI) {
    const raw = await window.__TAURI__.core.invoke('rpc', { body });
    return JSON.parse(raw);
  }
  const r = await fetch('/rpc', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body,
  });
  if (!r.ok) throw new EngineError('NETWORK', `HTTP ${r.status} from /rpc`);
  return r.json();
}

/** Typed engine error: code is the engine KIND (HTTP, TIMEOUT, INVALID_URL…) */
export class EngineError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

/** Call an engine method; throws EngineError with the engine's kind code. */
export async function api(method, params) {
  const j = await invokeRaw(method, params);
  if (j.error) {
    const msg = String(j.error.message || 'engine error ' + j.error.code);
    const m = msg.match(/^([A-Z_]+):\s*(.+)$/s);
    throw m ? new EngineError(m[1], m[2]) : new EngineError('ENGINE', msg);
  }
  return j.result;
}

export const engine = {
  info: () => api('engine.info'),
  list: () => api('download.list'),
  status: (id) => api('download.status', { id }),
  inspect: (url, cfg) => api('download.inspect', { url, ...(cfg || {}) }),
  create: (url, cfg) => api('download.create', { url, ...(cfg || {}) }),
  start: (id) => api('download.start', { id }),
  pause: (id) => api('download.pause', { id }),
  cancel: (id) => api('download.cancel', { id }),
  remove: (id, deleteFile = false) => api('download.remove', { id, delete_file: deleteFile }),
  verify: (id) => api('download.verify', { id }),
  merge: (videoId, audioId) => api('download.merge', { video_id: videoId, audio_id: audioId }),
};

/* ---------- desktop-only helpers ---------- */

/* Browser-ecosystem installer (browser-ecosystem turn): detection, one-click
 * setup for a selected Chromium browser, live integration status. */
export async function browserDetect() {
  try { return await window.__TAURI__.core.invoke('browser_detect'); }
  catch { return []; }
}
export async function extensionSetup(browserId, mode = 'guide') {
  return window.__TAURI__.core.invoke('extension_setup', { browser: browserId, mode });
}
export async function extensionStatus() {
  try { return await window.__TAURI__.core.invoke('extension_status'); }
  catch { return null; }
}

export async function defaultDownloadDir() {
  if (!TAURI) return null;
  try { return await window.__TAURI__.core.invoke('default_download_dir'); }
  catch { return null; }
}

/** Open the file itself. Scope-validated on the Rust side. */
export async function openFile(path) {
  if (!TAURI) throw new EngineError('UNAVAILABLE', 'Opening files requires the desktop app — path copied instead');
  await window.__TAURI__.core.invoke('open_path', { path, reveal: false });
}

/**
 * Native directory picker (desktop, tauri-plugin-dialog). Resolves to the
 * chosen absolute path, or null when cancelled / unavailable (browser preview).
 */
export async function pickDirectory(startPath) {
  if (!TAURI) return null;
  try {
    const picked = await window.__TAURI__.dialog.open({
      directory: true,
      multiple: false,
      defaultPath: startPath || undefined,
      title: 'Choose default download directory',
    });
    return typeof picked === 'string' && picked ? picked : null;
  } catch { return null; }
}

/** Reveal the containing folder. */
export async function revealFolder(path) {
  if (!TAURI) throw new EngineError('UNAVAILABLE', 'Opening folders requires the desktop app — path copied instead');
  await window.__TAURI__.core.invoke('open_path', { path, reveal: true });
}

/* ---------- settings persistence (desktop: JSON file via Rust; web: localStorage) ---------- */

const LS_KEY = 'velox.settings.v1';

export async function loadSettings(fallback) {
  if (TAURI) {
    try {
      const raw = await window.__TAURI__.core.invoke('settings_get');
      if (raw) return { ...fallback, ...JSON.parse(raw) };
    } catch { /* first run: file does not exist yet */ }
    return { ...fallback };
  }
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (raw) return { ...fallback, ...JSON.parse(raw) };
  } catch { /* private mode etc. */ }
  return { ...fallback };
}

export async function saveSettings(obj) {
  const json = JSON.stringify(obj, null, 2);
  if (TAURI) {
    await window.__TAURI__.core.invoke('settings_set', { json });
    return;
  }
  try { localStorage.setItem(LS_KEY, json); } catch { /* non-fatal */ }
}

/* ---------- events: names mirror engine-bridge::wire_event ---------- */

export const EVENT_NAMES = [
  'progress', 'retrying', 'completed', 'failed', 'verified',
  'queued', 'downloading', 'paused', 'cancelled', 'resolving', 'resolved',
  'merging', 'merged', 'removed',
];

/** Subscribe to engine events; returns an unsubscribe fn. */
export function subscribeEvents(handler) {
  if (TAURI) {
    const unsubs = [];
    for (const name of EVENT_NAMES) {
      window.__TAURI__.event.listen(name, (e) => handler(name, e.payload)).then((un) => unsubs.push(un));
    }
    return () => unsubs.forEach((u) => u());
  }
  const es = new EventSource('/events');
  let open = false;
  es.onopen = () => { open = true; handler('__open', {}); };
  es.onerror = () => handler(open ? '__resync' : '__error', {});
  for (const name of EVENT_NAMES) {
    es.addEventListener(name, (e) => {
      let data = {};
      try { data = JSON.parse(e.data); } catch { /* keep {} */ }
      handler(name, data);
    });
  }
  es.addEventListener('hello', (e) => {
    let d = {};
    try { d = JSON.parse(e.data); } catch { /* ignore */ }
    handler('__hello', d);
  });
  es.addEventListener('resync', () => handler('__resync', {}));
  return () => es.close();
}

export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
