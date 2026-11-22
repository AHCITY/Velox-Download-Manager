// Velox auto-update — the official signed Tauri updater flow.
// Check:  GitHub releases/latest/download/latest.json (version + minisign signature)
// Install: downloadAndInstall() verifies and runs the signed NSIS installer;
//          the app exits gracefully and the installer restarts the new version.
// Reminder: silent startup check, every 6h, and on window focus (throttled).
// State lives in store.update so screens + badges react through the normal
// notify() pipeline. In browser preview (no Tauri globals) everything reports
// honest unavailability.
//
// Updater reliability rules (Phase 3/4 repair, docs/PHASE0-ROOT-CAUSES.md RC7-RC9):
// 1. A version is ONLY offered when it is strictly NEWER than the running app
//    (semver compared in JS — the manifest alone can never produce a false or
//    repeated update).
// 2. The same release is announced once per install (localStorage-guarded),
//    not once per check.
// 3. Installing NEVER force-closes the app: the user confirms, active downloads
//    are paused first, and only then the signed installer runs — the app exits
//    itself and the installer brings the new version back up.
import { store, toast, pushActivity, notify } from './store.js';
import { engine } from './api.js';
import { confirmDialog } from './ui.js';
import { isNewerVersion } from './update-version.js';

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000; // 6 hours
const MIN_CHECK_GAP_MS = 30 * 60 * 1000;      // focus checks throttled to 30 min
const ANNOUNCE_KEY = 'velox.update.announced'; // last version the user was told about

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

// isNewerVersion lives in update-version.js (pure, unit-tested with node:test)
// and is re-exported for compat with existing importers.
export { isNewerVersion };

/** Number of downloads that would be interrupted by a restart right now. */
async function activeDownloadCount() {
  try {
    const rows = (await engine.list()) ?? [];
    return rows.filter((r) => ['downloading', 'retrying', 'merging', 'resolving'].includes(r.state)).length;
  } catch { return 0; } // engine unreachable → assume none; restart is still user-confirmed
}

/** Pause everything running (best effort, bounded). Returns rows that refused. */
async function pauseAllActive() {
  const rows = (await engine.list()) ?? [];
  const active = rows.filter((r) => ['downloading', 'retrying', 'resolving'].includes(r.state));
  const stuck = [];
  for (const r of active) {
    try { await engine.pause(r.id); } catch { stuck.push(r.id); }
  }
  return stuck;
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
      // RC8 guard: the plugin trusts the manifest's version claim. Semver truth
      // wins — an equal/older release is NEVER offered, no matter what the
      // manifest says (this is what produced the "update when current" bug).
      const current = await currentAppVersion();
      if (!isNewerVersion(u.version, current)) {
        pendingUpdate = null;
        setUpdateState({ status: 'uptodate', version: current, notes: '', at: lastCheckAt });
        return { status: 'uptodate' };
      }
      pendingUpdate = u;
      setUpdateState({ status: 'available', version: u.version, notes: u.body || '', date: u.date || '', at: lastCheckAt });
      // RC9 guard: announce each release ONCE per install, not once per check
      const verKey = `v${String(u.version).trim()}`;
      const announced = (() => { try { return localStorage.getItem(ANNOUNCE_KEY); } catch { return null; } })();
      if (announced !== verKey) {
        try { localStorage.setItem(ANNOUNCE_KEY, verKey); } catch { /* private mode */ }
        pushActivity(nowTs(), `<b>Update ${esc(u.version)}</b> is available — install it from Settings → Updates`);
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

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

/**
 * Download, verify (minisign, enforced by the plugin) and install.
 * PHASE 4 flow — never force-closes:
 *   1. active downloads are counted; the user confirms explicitly (or chooses
 *      to postpone) before anything is downloaded;
 *   2. active downloads are PAUSED and the pause is awaited;
 *   3. downloadAndInstall runs the signed installer, which WAITS for this app
 *      to exit, installs, and brings the new version back up;
 *   4. the app exits itself — no taskkill, no race, no lost bytes.
 * A failure at any point leaves the app running with an honest error state.
 */
export async function installUpdate() {
  const up = tauriUpdater();
  if (!up) return { ok: false, message: 'Updates require the desktop app' };
  if (!pendingUpdate) {
    // stale handle (state lost after reload): re-check first
    const res = await checkNow(true);
    if (res.status !== 'available') return { ok: false, message: 'No update is pending' };
  }

  // 1) explicit consent + active-download protection
  const active = await activeDownloadCount();
  let proceed = true;
  if (active > 0) {
    proceed = await confirmDialog({
      title: `Restart with ${active} active download${active === 1 ? '' : 's'}?`,
      message: `<b>${active}</b> download${active === 1 ? ' is' : 's are'} still running. Installing the update pauses ` +
        `them now (progress is checkpointed and resumes exactly where it stopped after the restart).<br><br>` +
        `Choose <b>Cancel</b> to keep downloading — the update offer stays in Settings → Updates.`,
      confirmLabel: 'Pause & install',
      danger: false,
    });
  } else {
    proceed = await confirmDialog({
      title: `Install Velox ${pendingUpdate.version}?`,
      message: 'The signed installer downloads, verifies and installs the update, then Velox restarts automatically.',
      confirmLabel: 'Install & restart',
      danger: false,
    });
  }
  if (!proceed) {
    toast('info', 'Update postponed', `Velox ${pendingUpdate.version} stays available in Settings → Updates.`, [], 6000);
    return { ok: false, message: 'postponed' };
  }

  // 2) pause active downloads and wait for the pauses to settle
  setUpdateState({ status: 'installing', got: 0, total: 0 });
  const stuck = await pauseAllActive();
  if (stuck.length) {
    setUpdateState({ status: 'error', error: `could not pause ${stuck.length} download(s)` });
    toast('err', 'Update postponed', `${stuck.length} download(s) refused to pause — nothing was closed. Try again in a moment.`, [], 9000);
    return { ok: false, message: 'pause failed' };
  }

  // 3) download + verify + run installer (it waits for this app to exit)
  try {
    await pendingUpdate.downloadAndInstall((ev) => {
      if (ev.event === 'Started') {
        setUpdateState({ total: ev.data?.contentLength ?? 0 });
      } else if (ev.event === 'Progress') {
        setUpdateState({ got: (store.update.got || 0) + (ev.data?.chunkLength ?? 0) });
      } else if (ev.event === 'Finished') {
        pushActivity(nowTs(), 'Update downloaded and verified — handing over to the installer…');
      }
    });
    // 4) the installer is waiting for this app to exit — comply gracefully.
    setUpdateState({ status: 'restarting' });
    pushActivity(nowTs(), 'Update installing — Velox restarts when the installer finishes…');
    const proc = tauriProcess();
    if (proc?.exit) setTimeout(() => { try { proc.exit(0); } catch { /* installer wait handles it */ } }, 600);
    else if (proc?.relaunch) setTimeout(() => { proc.relaunch().catch(() => {}); }, 600);
    return { ok: true };
  } catch (e) {
    setUpdateState({ status: 'error', error: String(e?.message || e).slice(0, 140), at: lastCheckAt });
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
