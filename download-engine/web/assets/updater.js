// Velox auto-update — the official signed Tauri updater flow.
// Check:  GitHub releases/latest/download/latest.json (version + minisign signature)
// Install: downloadAndInstall() runs the signed NSIS installer (passive mode);
//          relaunch() restarts the updated app afterwards.
// Reminder: silent startup check, every 6h, and on window focus (throttled).
// State lives in store.update so screens + badges react through the normal
// notify() pipeline. In browser preview (no Tauri globals) everything reports
// honest unavailability.
import { store, toast, pushActivity, notify } from './store.js';

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000; // 6 hours
const MIN_CHECK_GAP_MS = 30 * 60 * 1000;      // focus checks throttled to 30 min

let lastCheckAt = 0;
let checking = false;
let pendingUpdate = null; // the plugin's Update handle (downloadAndInstall lives here)

const nowTs = () => new Date().toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

function tauriUpdater() { return window.__TAURI__?.updater ?? null; }
function tauriProcess() { return window.__TAURI__?.process ?? null; }
function tauriApp() { return window.__TAURI__?.app ?? null; }

export function setUpdateState(patch) {
  store.update = { ...(store.update || { status: 'idle' }), ...patch };
  notify();
}

export async function currentAppVersion() {
  try { return (await tauriApp()?.getVersion?.()) ?? null; } catch { return null; }
}

/**
 * Check GitHub for a newer signed release.
 * announce=true surfaces errors (user-initiated "Check now"); background
 * checks stay quiet — an offline machine must never be nagged.
 */
export async function checkNow(announce = false) {
  const up = tauriUpdater();
  if (!up) {
    setUpdateState({ status: 'unavailable' });
    return { status: 'unavailable' };
  }
  if (checking) return { status: 'busy' };
  checking = true;
  setUpdateState({ status: 'checking' });
  try {
    const u = await up.check();
    lastCheckAt = Date.now();
    if (u && u.available) {
      const isNew = store.update?.version !== u.version;
      pendingUpdate = u;
      setUpdateState({ status: 'available', version: u.version, notes: u.body || '', date: u.date || '', at: lastCheckAt });
      if (isNew) {
        pushActivity(nowTs(), `<b>Update ${u.version}</b> is available — install it from Settings → Updates`);
        toast('info', 'Update available', `Velox ${u.version} — a newer version is ready to install.`, [
          { label: 'Install', act: () => installUpdate() },
          { label: 'Later', act: () => {} },
        ], 14000);
      }
      return { status: 'available', version: u.version };
    }
    pendingUpdate = null;
    setUpdateState({ status: 'uptodate', version: null, notes: '', at: lastCheckAt });
    return { status: 'uptodate' };
  } catch (e) {
    setUpdateState({ status: 'error', error: String(e?.message || e).slice(0, 140), at: lastCheckAt });
    if (announce) toast('warn', 'Update check failed', String(e?.message || e).slice(0, 90), [], 8000);
    return { status: 'error', message: String(e?.message || e) };
  } finally {
    checking = false;
    setUpdateState({});
  }
}

/** Download, verify (minisign, enforced by the plugin) and install; then relaunch. */
export async function installUpdate() {
  const up = tauriUpdater();
  if (!up) return { ok: false, message: 'Updates require the desktop app' };
  if (!pendingUpdate) return { ok: false, message: 'No update is pending' };
  setUpdateState({ status: 'installing', got: 0, total: 0 });
  try {
    await pendingUpdate.downloadAndInstall((ev) => {
      if (ev.event === 'Started') {
        setUpdateState({ total: ev.data?.contentLength ?? 0 });
      } else if (ev.event === 'Progress') {
        setUpdateState({ got: (store.update.got || 0) + (ev.data?.chunkLength ?? 0) });
      } else if (ev.event === 'Finished') {
        pushActivity(nowTs(), 'Update downloaded and verified — restarting…');
      }
    });
    pushActivity(nowTs(), 'Update installed — restarting…');
    setUpdateState({ status: 'restarting' });
    const proc = tauriProcess();
    if (proc?.relaunch) setTimeout(() => { proc.relaunch().catch(() => {}); }, 500);
    return { ok: true };
  } catch (e) {
    setUpdateState({ status: 'error', error: String(e?.message || e).slice(0, 140) });
    toast('err', 'Update failed', String(e?.message || e).slice(0, 90), [], 9000);
    return { ok: false, message: String(e?.message || e) };
  } finally {
    checking = false;
  }
}

export function initUpdater() {
  // capability check is the presence of the Tauri updater global — browser
  // preview without the desktop shell reports "unavailable" honestly
  if (!tauriUpdater()) {
    setUpdateState({ status: 'unavailable' });
    return;
  }
  setUpdateState({ status: 'idle' });
  // small startup reminder window; silent on failure (offline machines)
  setTimeout(() => checkNow(false), 4500);
  setInterval(() => checkNow(false), CHECK_INTERVAL_MS);
  window.addEventListener('focus', () => {
    if (Date.now() - lastCheckAt > MIN_CHECK_GAP_MS) checkNow(false);
  });
}
