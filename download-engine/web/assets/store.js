// Velox store — the single source of UI truth. The engine DB is authoritative;
// this store mirrors it, patches itself from live events, and re-syncs after
// reconnects. Screens subscribe to slices; progress events patch rows in place
// so a moving download never rerenders the whole app.
import { engine, subscribeEvents, loadSettings, saveSettings, EVENT_NAMES, friendlyEngineMessage } from './api.js';
import { planQueueStarts, setQueueHold } from './queue-plan.js';
import { orderRows, becameActive } from './ordering.js';

export const RUNNING_STATES = ['downloading', 'queued', 'created', 'retrying', 'resolving', 'merging'];
// Rows the engine pauses on request. A merge in progress is NOT pausable
// (ffmpeg has no resume — pause() refuses it honestly), so the UI must not
// offer Pause for it; finished/failed rows have nothing to stop.
export const PAUSABLE_STATES = ['downloading', 'retrying', 'queued', 'created'];
export const LISTENABLE_STATES = ['all', 'downloading', 'queued', 'completed', 'failed', 'paused', 'resolving', 'merging', 'cancelled'];
// Downloads screen filter segments: each maps to a set of engine states.
// "downloading" deliberately includes resolving/merging — they ARE active work.
// cancelled rows live under All only (their chip says so).
export const FILTER_STATES = {
  all: null,
  downloading: ['downloading', 'retrying', 'resolving', 'merging'],
  queued: ['queued', 'created'],
  paused: ['paused'],
  completed: ['completed'],
  failed: ['failed'],
};
// States that represent work IN FLIGHT right now (the Active Downloads
// indicator and the activity-first list ordering both use exactly this set;
// paused/queued/completed rows are not "active").
export const ACTIVE_STATES = FILTER_STATES.downloading; // downloading/retrying/resolving/merging

export const CATEGORIES = {
  video: ['mp4', 'mkv', 'webm', 'avi', 'mov', 'flv', 'wmv', 'm4v', 'ts', 'm3u8'],
  audio: ['mp3', 'm4a', 'flac', 'wav', 'ogg', 'opus', 'aac', 'wma'],
  image: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'ico', 'avif'],
  archive: ['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'zst', 'iso'],
  document: ['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'md', 'epub', 'csv'],
  software: ['exe', 'msi', 'dmg', 'deb', 'rpm', 'appimage', 'apk', 'pkg'],
  code: ['js', 'ts', 'py', 'rs', 'go', 'c', 'cpp', 'java', 'html', 'css', 'json', 'xml', 'yaml', 'sh'],
};

export function categoryOf(name) {
  const ext = String(name || '').split('.').pop().toLowerCase();
  for (const [cat, exts] of Object.entries(CATEGORIES)) {
    if (exts.includes(ext)) return cat;
  }
  return 'other';
}

/* ---------- formatting ---------- */
export function fmtBytes(n) {
  if (n == null || isNaN(n)) return '—';
  if (n < 1024) return `${n} B`;
  const u = ['KiB', 'MiB', 'GiB', 'TiB'];
  let v = n, i = -1;
  do { v /= 1024; i++; } while (v >= 1024 && i < u.length - 1);
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${u[i]}`;
}
export const fmtSpeed = (bps) => (bps && bps >= 1 ? `${fmtBytes(bps)}/s` : '—');
// Download intelligence: ETA from the SMOOTHED engine speed with honest states.
// Returns null only when truly incomputable (unknown size); stalled/unknown are
// surfaced by the caller instead of a misleading number.
export function etaState(r) {
  if (r.state === 'completed') return { label: 'done' };
  const total = r.total_size;
  if (!total) return { label: 'unknown size' };
  const bps = r.bps_smoothed || r.bps || 0;
  if (bps < 1024) return { label: 'stalled' };
  if (r.received >= total) return { label: 'done' };
  const secs = Math.round((total - r.received) / bps);
  if (secs < 1) return { label: 'done' };
  return { label: fmtEta(r.received, total, bps) || '—', secs };
}
export function fmtEta(received, total, bps) {
  if (!total || !bps || bps < 1 || received >= total) return null;
  const s = Math.round((total - received) / bps);
  if (s < 60) return `${s}s left`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s left`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m left`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h left`;
}
export function fmtTime(iso) {
  if (!iso) return '—';
  const d = new Date(iso);
  if (isNaN(d)) return iso;
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}
export function pct(r) {
  if (!r.total_size) return r.received > 0 ? null : 0;
  return Math.min(100, (100 * r.received) / r.total_size);
}

/* ---------- settings (persisted) ---------- */
export const DEFAULT_PREFS = {
  notifyCompleted: true,
  notifyFailed: true,
  confirmFileDelete: true,
  confirmRemove: false,
  density: 'comfortable',        // comfortable | compact
  animations: true,
  sidebarExpanded: true,
  openFolderOnComplete: false,
  theme: 'dark',                 // dark | light | purple | blue | <custom theme id>
  startDownloadsImmediately: true, // Add dialog: start right away vs add paused
  mediaFormat: 'mp4',            // default media format: mp4 | mp3
  mediaQuality: 'best',          // default quality: 'best' | height (480/720/…)
  defaultDir: '',               // default destination for NEW downloads (Settings → General)
  defaultConnections: '',       // '' = engine default; else int per download
  defaultOverwrite: false,      // pre-check "overwrite existing file" in the Add dialog
  // browser-setup onboarding state (version-aware; app.js maybeBrowserSetup):
  // the app version last evaluated, and the app version the user last dismissed
  browserSetupStateVersion: '',
  browserSetupDismissedFor: '',
};
export const DEFAULT_DIR_PREF = 'defaultDir';

export const store = {
  rows: new Map(),            // id -> StatusView (engine-authoritative)
  bps: new Map(),             // id -> last observed speed (from progress events)
  lastActive: new Map(),      // id -> ms when the row most recently ENTERED an active state
  engine: null,
  prefs: { ...DEFAULT_PREFS },
  selection: new Set(),
  filter: { state: 'all', query: '', category: 'all' },
  sort: { key: 'created_at', dir: 'desc' },
  activity: [],               // recent meaningful events, newest first
  toasts: [],                 // {id, kind, title, msg, actions}
  resolving: new Map(),       // url -> {state: 'resolving'|'resolved'|'failed', note}
  net: { aggBps: 0, active: 0, conns: 0, samples: [], peak: 0 },
  queues: [],                 // [{id, name, itemIds, maxConcurrent, active}]
  update: { status: 'idle' }, // auto-update state (updater.js)
  ready: false,
  connected: false,
};

/* ---------- pub/sub ---------- */
const subs = new Set();
export function subscribe(fn) { subs.add(fn); return () => subs.delete(fn); }
export function notify() {
  // setTimeout, not rAF: background/headless webviews throttle rAF to zero, which
  // silently froze all live updates; a 0-timer coalesces per tick and always runs
  if (notifyQueued) return;
  notifyQueued = true;
  setTimeout(() => { notifyQueued = false; subs.forEach((f) => f()); }, 0);
}
let notifyQueued = false;

/* ---------- settings ---------- */
export async function initPrefs() {
  store.prefs = await loadSettings(DEFAULT_PREFS);
}
let saveTimer = null;
// Preferences and queues share ONE settings document, and settings_set replaces
// the whole file — so every write has to carry both. A prefs-only write (any
// toggle in Settings) used to drop the `queues` key: every saved queue was gone
// at the next launch.
function writeLocalSettings() {
  clearTimeout(saveTimer);
  saveTimer = null;
  saveSettings({ ...store.prefs, queues: store.queues }).catch(() => { /* non-fatal */ });
}
export function setPrefs(patch) {
  store.prefs = { ...store.prefs, ...patch };
  notify();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(writeLocalSettings, 250);
}
export async function loadQueues() {
  const s = await loadSettings({});
  store.queues = Array.isArray(s.queues) ? s.queues : [];
}
export function saveQueues() {
  writeLocalSettings();
}

/* ---------- activity feed ---------- */
export function pushActivity(ts, html) {
  store.activity.unshift({ ts, html, at: Date.now() });
  if (store.activity.length > 200) store.activity.pop();
  notify();
}
const nowTs = () => new Date().toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

/* ---------- toasts ---------- */
let toastSeq = 0;
// completion burst-batcher (notification de-clutter): see the 'completed' case
const completedBurst = { names: [], ids: [], timer: null };
export function toast(kind, title, msg, actions = [], ttl = 6000) {
  // anti-clutter: at most 4 toasts visible; the oldest non-error one is dropped
  // first (errors are the ones a user may still need to act on)
  while (store.toasts.length >= 4) {
    const idx = store.toasts.findIndex((t) => t.kind !== 'err');
    store.toasts.splice(idx >= 0 ? idx : 0, 1);
  }
  const id = ++toastSeq;
  store.toasts.push({ id, kind, title, msg, actions });
  notify();
  setTimeout(() => dismissToast(id), ttl);
}
export function dismissToast(id) {
  const i = store.toasts.findIndex((t) => t.id === id);
  if (i >= 0) {
    const el = document.querySelector(`[data-toast="${id}"]`);
    if (el) { el.classList.add('out'); setTimeout(() => el.remove(), 240); }
    store.toasts.splice(i, 1);
    notify();
  }
}

/* ---------- data refresh ---------- */
export async function refreshAll() {
  try {
    const rows = await engine.list();
    const ids = new Set();
    for (const r of rows) {
      ids.add(r.id);
      const prev = store.rows.get(r.id);
      // activity-first ordering: stamp rows that (re)entered an active state,
      // from the authoritative resync as well as from live events
      if (becameActive(prev?.state, r.state, ACTIVE_STATES)) store.lastActive.set(r.id, Date.now());
      store.rows.set(r.id, r);
      if (prev && r.bps == null) r.bps = store.bps.get(r.id) ?? prev.bps ?? null;
    }
    for (const id of [...store.rows.keys()]) if (!ids.has(id)) { store.rows.delete(id); store.bps.delete(id); store.lastActive.delete(id); store.selection.delete(id); }
    if (!store.engine) {
      store.engine = await engine.info();
    }
    store.connected = true;
    store.ready = true;
    notify();
  } catch (e) {
    store.connected = false;
    notify();
  }
}

export async function refreshOne(id) {
  try {
    const fresh = await engine.status(id);
    if (fresh && fresh.id != null) { store.rows.set(id, fresh); store.bps.delete(id); }
    else { store.rows.delete(id); }
  } catch {
    const cur = store.rows.get(id);
    if (cur && !RUNNING_STATES.includes(cur.state)) store.rows.delete(id);
  }
  notify();
}

/* ---------- selectors ---------- */
export function visibleRows() {
  const { state, query, category } = store.filter;
  let rows = [...store.rows.values()];
  const group = FILTER_STATES[state];
  if (group) rows = rows.filter((r) => group.includes(r.state));
  if (category !== 'all') rows = rows.filter((r) => categoryOf(r.final_name) === category);
  if (query) {
    const q = query.toLowerCase();
    rows = rows.filter((r) =>
      (r.final_name || '').toLowerCase().includes(q) ||
      (r.url || '').toLowerCase().includes(q) ||
      (r.dest_dir || '').toLowerCase().includes(q));
  }
  const { key, dir } = store.sort;
  // activity-first ordering: active downloads on top (most recent activity
  // first), everything else under the user's chosen sort — see ordering.js
  return orderRows(rows, { activeStates: ACTIVE_STATES, lastActive: store.lastActive, sortKey: key, sortDir: dir });
}

export function counts() {
  const c = { all: store.rows.size, downloading: 0, queued: 0, completed: 0, failed: 0, paused: 0, resolving: 0, merging: 0, cancelled: 0 };
  for (const r of store.rows.values()) {
    const s = r.state;
    if (s === 'downloading' || s === 'created' || s === 'retrying') c.downloading++;
    else if (c[s] != null) c[s]++;
  }
  return c;
}

export function netSnapshot() {
  let agg = 0, conns = 0, active = 0;
  for (const r of store.rows.values()) {
    if (r.state === 'downloading') {
      active++;
      conns += r.connections_active || 0;
      agg += store.bps.get(r.id) || 0;
    }
  }
  return { agg, conns, active };
}

/* ---------- event ingestion ---------- */
const netTimer = setInterval(() => {
  const { agg, conns, active } = netSnapshot();
  store.net.aggBps = agg;
  store.net.conns = conns;
  store.net.active = active;
  store.net.samples.push({ t: Date.now(), bps: agg });
  if (store.net.samples.length > 180) store.net.samples.shift();
  store.net.peak = Math.max(store.net.peak, agg);
}, 1000);

export function stopNetTimer() { clearInterval(netTimer); }

let evUnsub = null;
export function startEvents() {
  if (evUnsub) evUnsub();
  evUnsub = subscribeEvents(async (name, d) => {
    if (name === '__open' || name === '__hello') {
      store.connected = true;
      await refreshAll();
      notify();
      return;
    }
    if (name === '__resync') { await refreshAll(); return; }
    if (name === '__error') { store.connected = false; notify(); return; }

    const row = d.id != null ? store.rows.get(d.id) : null;
    const prevState = row?.state;

    switch (name) {
      case 'progress': {
        if (!row) return;
        if (!RUNNING_STATES.includes(row.state)) return; // stale coalesced progress never resurrects a settled row
        row.received = d.received;
        if (d.total != null) row.total_size = d.total;
        if (d.segments != null) row.connections_active = d.segments;
        store.bps.set(d.id, d.bps || 0);
        row.bps = d.bps || 0;
        if (row.state === 'queued' || row.state === 'created' || row.state === 'resolving') row.state = 'downloading';
        break;
      }
      case 'completed': {
        store.bps.delete(d.id);
        if (row) { row.state = 'completed'; }
        await refreshOne(d.id);
        pushActivity(nowTs(), `<b>${esc(shortName(row?.final_name || `#${d.id}`))}</b> completed`);
        if (store.prefs.notifyCompleted) {
          // burst-batching: several completions within 1.5 s collapse into ONE
          // summary toast instead of a wall of near-identical popups
          completedBurst.names.push(row?.final_name || `#${d.id}`);
          completedBurst.ids.push(d.id);
          clearTimeout(completedBurst.timer);
          completedBurst.timer = setTimeout(() => {
            const names = completedBurst.names, ids = completedBurst.ids;
            completedBurst.names = []; completedBurst.ids = [];
            if (!names.length) return;
            if (names.length === 1) {
              toast('ok', 'Download completed', names[0], [
                { label: 'Open folder', act: () => window.__veloxActions?.reveal?.(ids[0]) },
              ]);
            } else {
              toast('ok', `${names.length} downloads completed`,
                names.slice(0, 5).join(', ') + (names.length > 5 ? ` +${names.length - 5} more` : ''), [], 7000);
            }
          }, 1500);
        }
        if (store.prefs.openFolderOnComplete) window.__veloxActions?.reveal?.(d.id);
        queueTick();
        break;
      }
      case 'failed': {
        store.bps.delete(d.id);
        await refreshOne(d.id);
        const msg = friendlyEngineMessage(d.message || 'download failed');
        pushActivity(nowTs(), `<b>${esc(shortName(row?.final_name || `#${d.id}`))}</b> failed — ${esc(msg)}`);
        if (store.prefs.notifyFailed) {
          toast('err', 'Download failed', `${row?.final_name || ''} ${msg}`.trim(), [
            { label: 'Retry', act: () => window.__veloxActions?.retry?.(d.id) },
            { label: 'Details', act: () => window.__veloxActions?.details?.(d.id) },
          ], 9000);
        }
        queueTick();
        break;
      }
      case 'verified': {
        await refreshOne(d.id);
        const name = row?.final_name || `#${d.id}`;
        // audit D6: `ok` only means "no mismatch detected". Only `matches` tells
        // whether a reference hash was actually compared (null = none existed:
        // the checksum was merely STORED). Older engines without the field fall
        // back to `ok`.
        const matches = d.matches === undefined ? (d.ok ? null : false) : d.matches;
        if (matches === false) {
          pushActivity(nowTs(), `Verification <b>FAILED</b> for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')}) — file does not match its checksum`);
          toast('err', 'Verification failed', `${name} · ${d.algo || ''} mismatch — the file may be corrupt`, [], 9000);
        } else if (matches === true) {
          pushActivity(nowTs(), `Verification passed for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')})`);
        } else {
          pushActivity(nowTs(), `Checksum computed for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')}) — no reference to compare`);
        } // successes go to the activity feed only; only FAILURES interrupt
        break;
      }
      case 'retrying': {
        if (row) row.state = 'retrying';
        pushActivity(nowTs(), `Retrying <b>${esc(shortName(row?.final_name || `#${d.id}`))}</b> — ${esc(d.reason || 'transient error')}`);
        break;
      }
      case 'resolving': {
        store.resolving.set(d.url, { state: 'resolving', note: '' });
        pushActivity(nowTs(), `Resolving media page…`);
        break;
      }
      case 'resolved': {
        store.resolving.set(d.url, { state: 'resolved', note: d.note || '' });
        pushActivity(nowTs(), `Media URL resolved — starting download`);
        setTimeout(() => { store.resolving.delete(d.url); }, 4000);
        break;
      }
      case 'merging': {
        if (row) row.state = 'merging';
        pushActivity(nowTs(), `Merging streams for <b>${esc(shortName(row?.final_name || `#${d.id}`))}</b>`);
        break;
      }
      case 'merged': {
        await refreshOne(d.id);
        pushActivity(nowTs(), `Merge finished for <b>${esc(shortName(row?.final_name || `#${d.id}`))}</b>`);
        break; // no toast: the completion burst already announced the result
      }
      case 'removed': {
        store.rows.delete(d.id);
        store.bps.delete(d.id);
        store.lastActive.delete(d.id);
        store.selection.delete(d.id);
        break;
      }
      case 'paused': case 'cancelled': case 'queued': case 'downloading': {
        if (row && row.state !== name) { row.state = name === 'downloading' && row.state === 'paused' ? 'paused' : name; }
        if (name === 'paused' || name === 'cancelled') store.bps.delete(d.id);
        await refreshOne(d.id);
        break;
      }
      default: break;
    }
    // stamp (re)entry into an active state from LIVE events too (resume,
    // queue start, retry): a resumed download jumps back to the top
    if (row && becameActive(prevState, row.state, ACTIVE_STATES)) {
      store.lastActive.set(d.id, Date.now());
    }
    notify();
  });
  return () => { if (evUnsub) evUnsub(); evUnsub = null; };
}

function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function shortName(name) {
  return String(name || '').length > 44 ? String(name).slice(0, 44) + '…' : name;
}

/* ---------- queue orchestrator ----------
   REAL orchestration through engine APIs (start/pause), driven by events —
   not timers that fake a scheduler. Honest limit: while the app is closed,
   nothing advances the queue (the engine has no scheduler contract yet). */
let queueBusy = false;
async function queueTick() {
  if (queueBusy) return;
  queueBusy = true;
  try {
    // an update install is in flight: rows were paused on purpose for the exit,
    // so the orchestrator must not undo that by starting waiting members
    const blocked = store.update?.status === 'installing' || store.update?.status === 'restarting';
    for (const id of planQueueStarts(store.queues, store.rows, { blocked })) {
      try { await engine.start(id); } catch { /* row may have finished already */ }
    }
  } finally {
    queueBusy = false;
  }
}
/** Re-run the orchestrator now (after assigning an item to a running queue). */
export const kickQueues = () => queueTick();
/* User pause/resume of a queue member. The orchestrator only advances items
 * that are waiting because of their queue, so a hand-paused member is recorded
 * in the queue's `held` list (persisted with the queues) until the user resumes
 * it — otherwise the next completion event or the next app start restarted it. */
export function holdQueueItem(id) {
  if (setQueueHold(store.queues, id, true)) saveQueues();
}
export function releaseQueueItem(id) {
  if (setQueueHold(store.queues, id, false)) saveQueues();
}
export function startQueueItem(qid) {
  const q = store.queues.find((x) => x.id === qid);
  if (!q) return;
  q.active = true;
  saveQueues();
  queueTick();
  notify();
}
export function pauseQueue(qid) {
  const q = store.queues.find((x) => x.id === qid);
  if (!q) return;
  q.active = false;
  saveQueues();
  for (const id of q.itemIds) {
    const r = store.rows.get(id);
    if (r && (r.state === 'downloading' || r.state === 'retrying')) engine.pause(id).catch(() => {});
  }
  notify();
}
export async function bootstrap() {
  await initPrefs();
  await loadQueues();
  await refreshAll();
  startEvents();
  queueTick();
  // safety net for missed events (e.g. webview throttling in background)
  setInterval(() => { if (document.visibilityState === 'visible' && store.connected) refreshAll(); }, 15000);
}
