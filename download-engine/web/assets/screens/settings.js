// Settings — every preference persists through the real settings store
// (JSON file on desktop, localStorage in browser) and takes effect immediately.
// Engine-fixed values are shown read-only with their actual values.
import { icon } from '../icons.js';
import { store, setPrefs, refreshAll, counts, toast } from '../store.js';
import { checkNow, installUpdate, currentAppVersion } from '../updater.js';
import { fmtBytes } from '../store.js';
import { isDesktop, defaultDownloadDir, pickDirectory } from '../api.js';
import { el, esc } from '../ui.js';

let built = false;
let section = 'general';

const SECTIONS = [
  ['general', 'General', 'settings'],
  ['updates', 'Updates', 'refresh'],
  ['downloads', 'Downloads', 'download'],
  ['network', 'Network', 'network'],
  ['browser', 'Browser', 'globe'],
  ['appearance', 'Appearance', 'eye'],
  ['advanced', 'Advanced', 'terminal'],
];

export function renderSettings(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-settings') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function toggleRow(name, title, desc, value, onChange) {
  const node = el(`<div class="setting-row">
    <div class="s-txt"><div class="s-name">${esc(title)}</div><div class="s-desc">${esc(desc)}</div></div>
    <div class="s-ctl"><label class="toggle"><input type="checkbox" ${value ? 'checked' : ''}><span class="tk"></span></label></div>
  </div>`);
  node.querySelector('input').onchange = (e) => onChange(e.target.checked);
  return node;
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-settings">
    <div class="screen-head"><div><h1>Settings</h1><div class="sub" id="seSub"></div></div></div>
    <div style="display:grid;grid-template-columns:190px 1fr;gap:18px" id="seGrid">
      <div class="card" style="padding:8px;align-self:start" id="seNav"></div>
      <div id="seBody"></div>
    </div>
  </div>`);
  return shell;
}

function paintShell(root) {
  root.querySelector('#seSub').textContent = 'Preferences persist on this machine and apply immediately';
  const nav = root.querySelector('#seNav');
  nav.innerHTML = SECTIONS.map(([k, l, ic]) =>
    `<button class="nav-item ${section === k ? 'active' : ''}" data-s="${k}">${icon(ic, 15)}<span class="nav-label">${l}</span></button>`).join('');
  nav.onclick = (e) => {
    const b = e.target.closest('[data-s]');
    if (b) { section = b.dataset.s; paintShell(root); }
  };
  const body = root.querySelector('#seBody');
  const card = el(`<div class="card" style="padding:6px 18px"></div>`);
  body.replaceChildren(card);
  const P = store.prefs;

  if (section === 'general') {
    card.append(
      toggleRow('notifyCompleted', 'Notify when a download completes', 'Shows a toast with an “Open folder” action.', P.notifyCompleted, (v) => setPrefs({ notifyCompleted: v })),
      toggleRow('notifyFailed', 'Notify when a download fails', 'Shows the failure reason with Retry / Details actions.', P.notifyFailed, (v) => setPrefs({ notifyFailed: v })),
      toggleRow('confirmFileDelete', 'Confirm before deleting files', 'Ask before permanently deleting a downloaded file from disk.', P.confirmFileDelete, (v) => setPrefs({ confirmFileDelete: v })),
      toggleRow('confirmRemove', 'Confirm before removing list entries', 'Ask before removing a download from the list (files are kept).', P.confirmRemove, (v) => setPrefs({ confirmRemove: v })),
      toggleRow('openFolderOnComplete', 'Open folder on completion', 'Reveals the containing folder as soon as a download finishes.', P.openFolderOnComplete, (v) => setPrefs({ openFolderOnComplete: v })),
      defaultDirRow(),
    );
  } else if (section === 'downloads') {
    card.append(
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Row density</div><div class="s-desc">Compact rows show more downloads per screen. Applies to the Downloads list.</div></div>
        <div class="s-ctl"><select id="seDensity"><option value="comfortable" ${P.density === 'comfortable' ? 'selected' : ''}>Comfortable</option><option value="compact" ${P.density === 'compact' ? 'selected' : ''}>Compact</option></select></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default connections per download</div><div class="s-desc">Pre-selected in the Add Download dialog. The engine clamps this to its per-host ceiling.</div></div>
        <div class="s-ctl"><select id="seDefConns">
          <option value="" ${!P.defaultConnections ? 'selected' : ''}>Engine default</option>
          ${[1, 2, 4, 8, 16].map((n) => `<option value="${n}" ${String(P.defaultConnections) === String(n) ? 'selected' : ''}>${n}</option>`).join('')}
        </select></div></div>`),
      toggleRow('defaultOverwrite', 'Overwrite existing files by default', 'Pre-checks "Overwrite existing file" in the Add dialog. Otherwise numbered names (file(1).ext) are chosen.', P.defaultOverwrite, (v) => setPrefs({ defaultOverwrite: v })),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Maximum concurrent downloads</div><div class="s-desc">Engine configuration — how many downloads run in parallel (fixed at startup).</div></div>
        <div class="s-ctl"><span class="tag acc">${store.engine?.max_concurrent_downloads ?? '—'}</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Connections per host</div><div class="s-desc">Engine ceiling for one host; per-download connection counts are clamped by it.</div></div>
        <div class="s-ctl"><span class="tag acc">${store.engine?.max_connections_per_host ?? '—'}</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Retry policy</div><div class="s-desc">Built into the engine: transient errors retry with backoff (stop-aware), permanent errors fail honestly with a reason.</div></div>
        <div class="s-ctl"><span class="tag ok">automatic</span></div></div>`),
    );
    const dens = card.querySelector('#seDensity');
    if (dens) dens.onchange = (e) => setPrefs({ density: e.target.value });
    const conns = card.querySelector('#seDefConns');
    if (conns) conns.onchange = (e) => setPrefs({ defaultConnections: e.target.value });
  } else if (section === 'network') {
    card.append(
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Global bandwidth limit</div><div class="s-desc">A global token bucket is not implemented in the engine. Per-download limits are enforced and can be set in the Add dialog.</div></div>
        <div class="s-ctl"><span class="tag warn">not available</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Connection limits</div><div class="s-desc">Per host: ${store.engine?.max_connections_per_host ?? '—'} · concurrent downloads: ${store.engine?.max_concurrent_downloads ?? '—'} (engine startup configuration).</div></div>
        <div class="s-ctl"><span class="tag acc">engine-managed</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Proxy configuration</div><div class="s-desc">The engine uses the system HTTP client defaults; an explicit proxy UI is not implemented.</div></div>
        <div class="s-ctl"><span class="tag warn">not available</span></div></div>`),
    );
  } else if (section === 'browser') {
    card.append(
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Browser extension</div><div class="s-desc">Chromium-based browsers: the Velox extension sends downloads from web pages to the app (right-click, then Download with Velox). Install and connect it from the Browser screen; Firefox is not supported yet.</div></div>
        <div class="s-ctl"><button class="btn sm" id="seGoBrowser">Open Browser</button></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Clipboard monitoring</div><div class="s-desc">Not implemented; the webview cannot watch the clipboard without an OS-level backend.</div></div>
        <div class="s-ctl"><span class="tag warn">not available</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Media detection</div><div class="s-desc">Automatic via the yt-dlp extractor when you paste a media page (YouTube etc.).</div></div>
        <div class="s-ctl"><span class="tag ${store.engine?.merge_capable ? 'ok' : 'warn'}">${store.engine?.merge_capable ? 'ready' : 'needs yt-dlp'}</span></div></div>`),
    );
    // the row used to claim the extension was "not implemented"; it ships with
    // the installer and the Browser screen owns the install flow
    card.querySelector('#seGoBrowser').onclick = () => { location.hash = '#browser'; };
  } else if (section === 'appearance') {
    card.append(
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Theme</div><div class="s-desc">Velox is deliberately dark-only.</div></div>
        <div class="s-ctl"><span class="tag acc">dark</span></div></div>`),
      toggleRow('animations', 'Interface animations', 'Subtle transitions on dialogs, rows and screen changes.', P.animations, (v) => { setPrefs({ animations: v }); document.body.dataset.anim = v ? 'on' : 'off'; }),
      toggleRow('sidebarExpanded', 'Expanded sidebar', 'Show section labels; turn off for a compact icon rail.', P.sidebarExpanded, (v) => { setPrefs({ sidebarExpanded: v }); document.dispatchEvent(new CustomEvent('velox:sidebar', { detail: v })); }),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Keyboard shortcuts</div><div class="s-desc">
        <span class="kbd">Ctrl N</span> add download &nbsp; <span class="kbd">Ctrl V</span> paste URL &nbsp; <span class="kbd">Ctrl F</span> search &nbsp; <span class="kbd">Ctrl A</span> select all &nbsp; <span class="kbd">Space</span> pause/resume selection &nbsp; <span class="kbd">Del</span> remove selection &nbsp; <span class="kbd">Esc</span> close dialog</div></div></div>`),
    );
  } else if (section === 'updates') {
    const st = store.update || { status: 'idle' };
    const rows = [];
    const ver = currentAppVersion();
    rows.push(el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Installed version</div><div class="s-desc">Velox desktop app (the updater compares this against the latest signed release on GitHub).</div></div>
      <div class="s-ctl"><span class="tag acc" id="seAppVer">…</span></div></div>`));
    const updRow = el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Update status</div><div class="s-desc" id="seUpdDesc"></div></div>
      <div class="s-ctl" id="seUpdCtl"></div></div>`);
    rows.push(updRow);
    rows.push(el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Check for updates</div><div class="s-desc">Checks GitHub for a newer signed release. Runs silently at startup, every 6 hours, and when the window regains focus.</div></div>
      <div class="s-ctl"><button class="btn sm" id="seCheckNow">${icon('refresh', 12)} Check now</button></div></div>`));
    rows.forEach((r) => card.append(r));

    const paint = () => {
      const st2 = store.update || { status: 'idle' };
      const ctl = updRow.querySelector('#seUpdCtl');
      const desc = updRow.querySelector('#seUpdDesc');
      const verEl = card.querySelector('#seAppVer');
      if (verEl && ver) ver.then((v) => { if (v) verEl.textContent = 'v' + v; });
      if (st2.status === 'available') {
        desc.innerHTML = `<b>Velox ${esc(st2.version)}</b> is available.` + (st2.notes ? ` <span style="color:var(--t3)">${esc(String(st2.notes).slice(0, 160))}</span>` : '');
        ctl.innerHTML = `<button class="btn primary sm" id="seInstall">${icon('download', 12)} Install ${esc(st2.version)}</button>`;
        ctl.querySelector('#seInstall').onclick = () => { installUpdate(); paint(); };
      } else if (st2.status === 'installing') {
        const pct = st2.total ? Math.min(100, Math.round((100 * st2.got) / st2.total)) : null;
        desc.textContent = pct == null ? 'Downloading and verifying the signed installer…' : `Downloading — ${pct}% of ${fmtBytes(st2.total)}`;
        ctl.innerHTML = `<span class="btn-spinner" style="border-color:var(--accent-bg);border-top-color:var(--accent)"></span>`;
      } else if (st2.status === 'restarting') {
        desc.textContent = 'Installing — Velox exits now and the installer brings the new version back up.';
        ctl.innerHTML = '';
      } else if (st2.status === 'checking') {
        desc.textContent = 'Checking GitHub for a newer release…';
        ctl.innerHTML = `<span class="btn-spinner" style="border-color:var(--accent-bg);border-top-color:var(--accent)"></span>`;
      } else if (st2.status === 'uptodate') {
        const at = st2.at ? ` Last checked ${new Date(st2.at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}.` : '';
        desc.textContent = `You are on the latest version.${at}`;
        ctl.innerHTML = '<span class="tag ok">up to date</span>';
      } else if (st2.status === 'error') {
        desc.textContent = `Last check failed: ${st2.error || 'unknown error'}`;
        ctl.innerHTML = '<span class="tag err">error</span>';
      } else if (st2.status === 'unavailable') {
        desc.textContent = 'Auto-update requires the desktop app. In the browser preview, download the installer from GitHub manually.';
        ctl.innerHTML = '<span class="tag">desktop only</span>';
      } else {
        desc.textContent = 'No check has run yet in this session.';
        ctl.innerHTML = '<span class="tag">idle</span>';
      }
    };
    paint();
    card.querySelector('#seCheckNow').onclick = () => { checkNow(true).then(() => paint()); };
    card.addEventListener('velox:update', paint);
  } else if (section === 'advanced') {
    const c = counts();
    card.append(
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Engine version</div><div class="s-desc">${esc(store.engine?.name || 'velox')} engine, JSON-RPC API v${esc(store.engine?.api_version || '?')}</div></div>
        <div class="s-ctl"><span class="tag acc">v${esc(store.engine?.version || '?')}</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Merge capability</div><div class="s-desc">ffmpeg on PATH enables the media merge stage (LGPL builds only).</div></div>
        <div class="s-ctl"><span class="tag ${store.engine?.merge_capable ? 'ok' : 'warn'}">${store.engine?.merge_capable ? 'available' : 'unavailable'}</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Database</div><div class="s-desc">${esc(store.engine?.data_dir || '—')} — ${c.all} rows: ${c.completed} completed, ${c.failed} failed, ${c.paused} paused, ${c.downloading + c.queued} active.</div></div>
        <div class="s-ctl"><button class="btn sm" id="seRefresh">${icon('refresh', 12)} Refresh</button></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Connection state</div><div class="s-desc">Live events from the engine; the UI re-syncs authoritative state on reconnect.</div></div>
        <div class="s-ctl"><span class="tag ${store.connected ? 'ok' : 'err'}">${store.connected ? 'connected' : 'disconnected'}</span></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Event log</div><div class="s-desc">Last engine events seen by this session (diagnostics).</div></div>
        <div class="s-ctl"><button class="btn sm" id="seCopyLog">${icon('copy', 12)} Copy</button></div></div>`),
    );
    card.querySelector('#seRefresh').onclick = () => refreshAll();
    card.querySelector('#seCopyLog').onclick = () => {
      const log = store.activity.map((a) => `${a.ts} ${a.html.replace(/<[^>]+>/g, '')}`).join('\n');
      navigator.clipboard.writeText(log).then(() => toast('ok', 'Event log copied', ''), () => {});
    };
  }
}
// Default download directory: a REAL native folder picker on desktop; the
// choice becomes the pre-filled destination for NEW downloads (Add dialog)
// and persists across restarts. The engine's startup dir remains the fallback.
function defaultDirRow() {
  const eff = store.prefs.defaultDir || store.engine?.default_out_dir || '';
  const row = el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default download directory</div>
      <div class="s-desc">Pre-filled destination for new downloads. New downloads land here unless you change the folder in the Add dialog.</div>
      <div class="s-desc" style="font-family:var(--mono);font-size:11px;margin-top:4px;word-break:break-all" id="seDirVal"></div></div>
      <div class="s-ctl"><button class="btn sm" id="sePickDir">${icon('folderOpen', 13)} Choose…</button></div></div>`);
  const val = row.querySelector('#seDirVal');
  const paint = (d) => { val.textContent = d || '—'; val.title = d || ''; };
  paint(eff);
  row.querySelector('#sePickDir').onclick = async () => {
    const chosen = await pickDirectory(store.prefs.defaultDir || store.engine?.default_out_dir || '');
    if (!chosen) return; // cancelled
    setPrefs({ defaultDir: chosen });
    paint(chosen);
    toast('ok', 'Default directory updated', chosen);
  };
  if (isDesktop && !store.prefs.defaultDir) {
    // desktop first run: show the OS dir the engine would use
    defaultDownloadDir().then((d) => { if (d && !store.prefs.defaultDir) paint(d); });
  }
  return row;
}
export { renderSettings as render };
