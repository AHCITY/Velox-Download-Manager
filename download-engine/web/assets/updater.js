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
let installing = false; // an install is in flight: no second install, no status overwrites

const nowTs = () => new Date().toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

function tauriUpdater() { return window.__TAURI__?.updater ?? null; }
function tauriProcess() { return window.__TAURI__?.process ?? null; }
function tauriApp() { return window.__TAURI__?.app ?? null; }

export function setUpdateState(patch) {
  store.update = { ...(store.update || { status: 'idle' }), ...patch };
  notify();
  // Settings → Updates repaints on this event (store.notify only repaints the
  // ACTIVE screen's live regions — Settings' update card used to go stale until
  // the user clicked something)
  try { document.dispatchEvent(new CustomEvent('velox:update')); } catch { /* non-fatal */ }
}

export async function currentAppVersion() {
  try { return (await tauriApp()?.getVersion?.()) ?? null; } catch { return null; }
}

// isNewerVersion lives in update-version.js (pure, unit-tested with node:test)
// and is re-exported for compat with existing importers.
export { isNewerVersion };

/** Downloads a restart would interrupt right now; `merging` of them cannot be paused. */
async function activeDownloads() {
  try {
    const rows = (await engine.list()) ?? [];
    const active = rows.filter((r) => ['downloading', 'retrying', 'merging', 'resolving'].includes(r.state));
    return { count: active.length, merging: active.filter((r) => r.state === 'merging').length };
  } catch { return { count: 0, merging: 0 }; } // engine unreachable: assume none; restart is still user-confirmed
}

/**
 * Pause everything pausable (best effort, bounded). Returns the rows that
 * refused (`stuck`) and the ones this call paused (`paused`): when the install
 * does not happen, the latter are handed back because the app keeps running.
 */
async function pauseAllActive() {
  const rows = (await engine.list()) ?? [];
  const active = rows.filter((r) => ['downloading', 'retrying', 'resolving'].includes(r.state));
  const stuck = [];
  const paused = [];
  for (const r of active) {
    try { await engine.pause(r.id); paused.push(r.id); } catch { stuck.push(r.id); }
  }
  return { stuck, paused };
}

async function resumeRows(ids) {
  for (const id of ids) {
    try { await engine.start(id); } catch { /* row settled meanwhile */ }
  }
}

/**
 * Check GitHub for a newer signed release.
 * announce=true surfaces errors (user-initiated "Check now"); background
 * checks stay quiet — an offline machine must never be nagged.
 */
export async function checkNow(announce = false) {
  // the focus timer, the 6h timer and the Settings button all land here: while
  // an install is running they must not overwrite "installing"/"restarting"
  if (installing) return { status: 'installing' };
  return runCheck(announce);
}

async function runCheck(announce = false) {
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
  // re-entrancy: the toast button, Settings and the app menu all reach here, and
  // two installs racing each other would launch two installers
  if (installing) return { ok: false, message: 'an update is already installing' };
  installing = true;
  try {
    return await runInstall();
  } finally {
    // after a successful hand-over the app is exiting and the flag stays set, so
    // nothing can overwrite the "restarting" state in its last moments
    if (store.update?.status !== 'restarting') installing = false;
  }
}

async function runInstall() {
  const up = tauriUpdater();
  if (!up) return { ok: false, message: 'Updates require the desktop app' };
  if (!pendingUpdate) {
    // stale handle (state lost after reload): re-check first
    const res = await runCheck(true);
    if (res.status !== 'available') return { ok: false, message: 'No update is pending' };
  }

  // 1) explicit consent + active-download protection
  const { count: active, merging } = await activeDownloads();
  let proceed = true;
  if (active > 0) {
    // a merge cannot be paused and the exit interrupts it: the row comes back as
    // failed (merge interrupted) and needs Retry. Say so instead of promising a
    // seamless resume for it.
    const m = merging === 1;
    const mergeNote = merging
      ? `<br><br><b>${merging} download${m ? ' is' : 's are'} still merging</b> (streams being combined). A merge cannot be paused: ` +
        `the restart interrupts ${m ? 'it' : 'them'}, ${m ? 'it' : 'they'} will show as <b>failed</b> (merge interrupted) and ` +
        `${m ? 'needs' : 'need'} <b>Retry</b> after the update. Cancel to let the merge finish first.`
      : '';
    proceed = await confirmDialog({
      title: `Restart with ${active} active download${active === 1 ? '' : 's'}?`,
      message: `<b>${active}</b> download${active === 1 ? ' is' : 's are'} still running. Installing the update pauses ` +
        (merging ? `the pausable ones now (progress is checkpointed and resumes exactly where it stopped after the restart).` : `them now (progress is checkpointed and resumes exactly where it stopped after the restart).`) +
        `${mergeNote}<br><br>` +
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
  const { stuck, paused } = await pauseAllActive();
  if (stuck.length) {
    await resumeRows(paused); // the app stays running: give back what was taken
    setUpdateState({ status: 'error', error: `could not pause ${stuck.length} download(s)` });
    toast('err', 'Update postponed', `${stuck.length} download(s) refused to pause — nothing was closed. Try again in a moment.`, [], 9000);
    return { ok: false, message: 'pause failed' };
  }

  // 3) download + verify + run installer (it waits for this app to exit).
  // Record the attempt FIRST: on the next launch we compare versions and
  // honestly report "did not install" if the installer never completed
  // (the tauri-plugin-updater #3514 failure mode — app exits, nothing installs).
  try {
    localStorage.setItem('velox.update.attempt', JSON.stringify({
      from: (await currentAppVersion()) || '',
      to: pendingUpdate.version,
      at: Date.now(),
    }));
  } catch { /* private mode */ }
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
    // 4) Windows: downloadAndInstall() already exited the process INSIDE the
    // plugin command (ShellExecuteW "/P /UPDATE /R" then std::process::exit) —
    // the JS promise never resolves there, and this code is unreachable on
    // Windows. The old 600ms exit(0) raced the installer spawn and produced
    // "app closes, update never installs". Reaching here means macOS/Linux:
    // relaunch directly, no competing timer.
    setUpdateState({ status: 'restarting' });
    pushActivity(nowTs(), 'Update installing — Velox restarts when the installer finishes…');
    const proc = tauriProcess();
    if (proc?.relaunch) { try { await proc.relaunch(); } catch { /* already exiting */ } }
    return { ok: true };
  } catch (e) {
    await resumeRows(paused); // the install failed and the app keeps running
    setUpdateState({ status: 'error', error: String(e?.message || e).slice(0, 140), at: lastCheckAt });
    toast('err', 'Update failed', String(e?.message || e).slice(0, 90), [], 9000);
    return { ok: false, message: String(e?.message || e) };
  } finally {
    checking = false;
  }
}

export async function initUpdater() {
  // capability check is the presence of the Tauri updater global — browser
  // preview without the desktop shell reports "unavailable" honestly
  if (!tauriUpdater()) {
    setUpdateState({ status: 'unavailable' });
    return;
  }
  setUpdateState({ status: 'idle' });
  // verify-on-next-launch: an attempt was recorded but this app is still the OLD
  // version -> the installer never completed. Say so once, honestly.
  try {
    const raw = localStorage.getItem('velox.update.attempt');
    if (raw) {
      const a = JSON.parse(raw);
      const cur = await currentAppVersion();
      if (cur && a?.to && cur !== a.to && isNewerVersion(a.to, cur)) {
        localStorage.removeItem('velox.update.attempt');
        setUpdateState({ status: 'error', error: `The update to ${a.to} did not finish installing` });
        toast('warn', 'Update not installed',
          `Velox is still on ${cur} — the update to ${a.to} did not complete. Try again from Settings → Updates.`, [], 10000);
      } else if (cur && a?.to && cur === a.to) {
        localStorage.removeItem('velox.update.attempt'); // install succeeded
      }
    }
  } catch { /* non-fatal */ }
  // small startup reminder window; silent on failure (offline machines)
  setTimeout(() => checkNow(false), 4500);
  setInterval(() => checkNow(false), CHECK_INTERVAL_MS);
  window.addEventListener('focus', () => {
    if (Date.now() - lastCheckAt > MIN_CHECK_GAP_MS) checkNow(false);
  });
}
