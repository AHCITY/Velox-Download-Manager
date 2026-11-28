// Settings — every preference persists through the real settings store
// (JSON file on desktop, localStorage in browser) and takes effect immediately.
// Engine-fixed values are shown read-only with their actual values.
import { icon } from '../icons.js';
import { store, setPrefs, refreshAll, counts, toast } from '../store.js';
import { checkNow, installUpdate, currentAppVersion } from '../updater.js';
import { fmtBytes } from '../store.js';
import { isDesktop, defaultDownloadDir, pickDirectory } from '../api.js';
import { el, esc } from '../ui.js';
import { BUILTIN_THEMES, customThemeList, applyTheme, registerCustomTheme, unregisterCustomTheme, sanitizeTheme, listInstalledThemes, getTheme, listCommunitySources, fetchCommunityCatalog, savedCommunityEntries, isCommunitySaved, saveCommunityTheme, removeSavedCommunityTheme, previewTheme, endThemePreview } from '../themes.js';

let built = false;
let section = 'general';

const SECTIONS = [
  ['general', 'General', 'settings'],
  ['updates', 'Updates', 'refresh'],
  ['downloads', 'Downloads', 'download'],
  ['media', 'Media', 'fileVideo'],
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
    <div class="se-grid" id="seGrid">
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
      toggleRow('startDownloadsImmediately', 'Start downloads immediately', 'When unchecked, new downloads are added paused so you can start them in bulk.', P.startDownloadsImmediately, (v) => setPrefs({ startDownloadsImmediately: v })),
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
      themeRow(),
      toggleRow('animations', 'Interface animations', 'Subtle transitions on dialogs, rows and screen changes.', P.animations, (v) => { setPrefs({ animations: v }); document.body.dataset.anim = v ? 'on' : 'off'; }),
      toggleRow('sidebarExpanded', 'Expanded sidebar', 'Show section labels; turn off for a compact icon rail.', P.sidebarExpanded, (v) => { setPrefs({ sidebarExpanded: v }); document.dispatchEvent(new CustomEvent('velox:sidebar', { detail: v })); }),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Keyboard shortcuts</div><div class="s-desc">
        <span class="kbd">Ctrl N</span> add download &nbsp; <span class="kbd">Ctrl V</span> paste URL &nbsp; <span class="kbd">Ctrl F</span> search &nbsp; <span class="kbd">Ctrl A</span> select all &nbsp; <span class="kbd">Space</span> pause/resume selection &nbsp; <span class="kbd">Del</span> remove selection &nbsp; <span class="kbd">Esc</span> close dialog</div></div></div>`),
    );
  } else if (section === 'media') {
    card.append(
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default media format</div><div class="s-desc">Pre-selected in the Add dialog for YouTube and other media pages. MP4 downloads video + audio; MP3 extracts audio only (requires the bundled FFmpeg).</div></div>
        <div class="s-ctl"><select id="seMediaFormat">
          <option value="mp4" ${P.mediaFormat !== 'mp3' ? 'selected' : ''}>MP4 — video + audio</option>
          <option value="mp3" ${P.mediaFormat === 'mp3' ? 'selected' : ''}>MP3 — audio only</option>
        </select></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default video quality</div><div class="s-desc">Pre-selected quality for MP4 downloads. The list always shows the qualities the source actually offers — this is only the default when a higher one exists.</div></div>
        <div class="s-ctl"><select id="seMediaQuality">
          <option value="best" ${P.mediaQuality === 'best' ? 'selected' : ''}>Best available</option>
          ${[2160, 1440, 1080, 720, 480, 360].map((h) => `<option value="${h}" ${String(P.mediaQuality) === String(h) ? 'selected' : ''}>${h}p${h >= 2160 ? ' (4K)' : h === 1440 ? ' (2K)' : ''}</option>`).join('')}
        </select></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Media tooling</div><div class="s-desc">yt-dlp resolves media pages; FFmpeg merges streams (MP4) and extracts MP3. Both are bundled with the offline installer and fetched by the online installer when selected.</div></div>
        <div class="s-ctl"><span class="tag ${store.engine?.merge_capable ? 'ok' : 'warn'}">${store.engine?.merge_capable ? 'ready' : 'needs yt-dlp / FFmpeg'}</span></div></div>`),
    );
    const fmt = card.querySelector('#seMediaFormat');
    if (fmt) fmt.onchange = (e) => setPrefs({ mediaFormat: e.target.value });
    const q = card.querySelector('#seMediaQuality');
    if (q) q.onchange = (e) => setPrefs({ mediaQuality: e.target.value });
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
// Theme picker — built-in swatches + user-installed third-party themes.
// Custom themes are sanitized JSON manifests (assets/themes.js): they can
// only set known color variables and are contrast-checked on install.
function themeRow() {
  const row = el(`<div class="setting-row"><div class="s-txt">
      <div class="s-name">Theme</div>
      <div class="s-desc">Applies immediately and persists across restarts. Third-party themes (.json) can be installed below, or browse the online community catalog.</div>
      <div class="theme-swatches" id="seThemeSw" style="margin-top:10px"></div>
      <div id="seThemeCustom" style="margin-top:10px"></div>
      <div id="seThemeErr" class="s-desc" style="color:var(--err);margin-top:6px"></div>
    </div>
    <div class="s-ctl" style="flex-direction:column;align-items:flex-end;gap:8px">
      <button class="btn sm" id="seThemeCommunity">${icon('globe', 12)} Community themes</button>
      <button class="btn sm ghost" id="seThemeInstall">${icon('plus', 12)} Install theme…</button>
      <button class="btn sm ghost" id="seThemeReload">${icon('refresh', 12)} Refresh list</button>
    </div></div>`);

  const swatches = row.querySelector('#seThemeSw');
  const customBox = row.querySelector('#seThemeCustom');
  const errBox = row.querySelector('#seThemeErr');

  const swatch = (t) => {
    const b = el(`<button class="theme-sw ${store.prefs.theme === t.id ? 'on' : ''}" data-theme-id="${esc(t.id)}"
      title="${esc(t.name)}" aria-label="Theme ${esc(t.name)}">
      <span class="sw-preview" style="background:${t.colors['--bg1']}">
        <span class="sw-bar" style="background:${t.colors['--accent']}"></span>
        <span class="sw-bar" style="background:${t.colors['--ok']};width:60%"></span>
      </span>
      <span class="sw-name">${esc(t.name)}</span>
    </button>`);
    b.onclick = () => {
      if (applyTheme(t.id)) { setPrefs({ theme: t.id }); errBox.textContent = ''; paint(); }
      else errBox.textContent = `“${t.name}” failed the contrast check and was not applied.`;
    };
    return b;
  };

  const paint = () => {
    swatches.replaceChildren(...Object.values(BUILTIN_THEMES).map(swatch));
    const customs = customThemeList();
    customBox.innerHTML = customs.length
      ? `<div class="s-desc" style="margin-bottom:6px">Installed themes</div>`
      : `<div class="s-desc">No third-party themes installed. Install a .json theme file (structure: docs/THEMES.md).</div>`;
    for (const t of customs) {
      const b = swatch(t);
      const rm = el(`<button class="theme-rm" title="Remove theme" aria-label="Remove ${esc(t.name)}">${icon('x', 11)}</button>`);
      rm.onclick = async (e) => {
        e.stopPropagation();
        try { if (window.__TAURI__?.core?.invoke) await window.__TAURI__.core.invoke('theme_uninstall', { id: t.id }); } catch { /* browser preview */ }
        if (store.prefs.theme === t.id) setPrefs({ theme: 'dark' });
        unregisterCustomTheme(t.id);
        await listInstalledThemes();
        if (applyTheme(store.prefs.theme || 'dark')) { /* refreshed */ }
        paint();
      };
      b.style.position = 'relative';
      b.appendChild(rm);
    }
    // mark selection state on freshly painted buttons
    swatches.querySelectorAll('.theme-sw').forEach((b) =>
      b.classList.toggle('on', b.dataset.themeId === store.prefs.theme));
    customBox.querySelectorAll('.theme-sw').forEach((b) =>
      b.classList.toggle('on', b.dataset.themeId === store.prefs.theme));
  };
  paint();

  row.querySelector('#seThemeInstall').onclick = async () => {
    // desktop: native file open via the dialog plugin; browser preview: paste
    if (window.__TAURI__?.core?.invoke) {
      try {
        // The native picker and file read stay inside Rust. The renderer never
        // passes an arbitrary local path into a filesystem-reading command.
        const txt = await window.__TAURI__.core.invoke('theme_pick_file');
        if (txt == null) return;
        await installThemeJson(txt, errBox, paint);
      } catch (e) {
        errBox.textContent = `Could not read the theme file: ${e}`;
      }
    } else {
      errBox.textContent = 'Theme installation runs in the desktop app (browser preview cannot read files).';
    }
  };
  row.querySelector('#seThemeReload').onclick = async () => {
    await listInstalledThemes();
    paint();
  };
  row.querySelector('#seThemeCommunity').onclick = () => openCommunityThemes({ repaint: paint });
  return row;
}

async function installThemeJson(txt, errBox, repaint) {
  let manifest;
  try { manifest = JSON.parse(txt); } catch { errBox.textContent = 'Not valid JSON.'; return; }
  const res = sanitizeTheme(manifest);
  if (!res.ok) { errBox.textContent = 'Theme rejected: ' + res.errors.slice(0, 3).join('; '); return; }
  if (window.__TAURI__?.core?.invoke) {
    // In the desktop app, persistence must succeed before the theme is
    // registered or selected; never report a theme as installed on failure.
    await window.__TAURI__.core.invoke('theme_install', { json: txt });
  }
  registerCustomTheme(res.theme);
  if (applyTheme(res.theme.id)) {
    setPrefs({ theme: res.theme.id });
    toast('ok', 'Theme installed', res.theme.name);
  }
  errBox.textContent = '';
  repaint();
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
/* Community themes popup — Appearance → Community themes.
 * Sources ship EMPTY for now (COMMUNITY_SOURCES in assets/themes.js): the
 * modal, live preview and download flow are fully functional; pointing
 * COMMUNITY_SOURCES at a registry lights the catalog up later. Downloaded
 * themes persist in the browser and in the desktop themes folder, then show
 * up in the picker above like any installed theme. The whole dialog uses the
 * shared .overlay/.dialog primitives and theme variables — it inherits every
 * theme automatically. */
function openCommunityThemes({ repaint }) {
  document.querySelectorAll('.ct-overlay').forEach((n) => n.remove());
  const overlay = el(`<div class="overlay ct-overlay show"><div class="dialog" style="max-width:680px" role="dialog" aria-modal="true" aria-label="Community themes">
    <div class="dialog-head"><h3 style="display:flex;align-items:center;gap:8px">${icon('globe', 14)} Community themes</h3>
      <button class="theme-rm" id="ctClose" title="Close" aria-label="Close" style="position:static;width:24px;height:24px">${icon('x', 13)}</button></div>
    <div class="dialog-body">
      <div id="ctBanner"></div>
      <div class="field" style="margin-bottom:10px"><input type="text" id="ctSearch" placeholder="Search community themes…" autocomplete="off"></div>
      <div id="ctList"></div>
    </div>
    <div class="dialog-foot" style="justify-content:space-between;align-items:center;gap:10px">
      <span class="s-desc" id="ctStatus" style="flex:1"></span>
      <button class="btn sm ghost" id="ctRefresh">${icon('refresh', 12)} Refresh</button>
    </div>
  </div></div>`);
  document.body.appendChild(overlay);
  const list = overlay.querySelector('#ctList');
  const banner = overlay.querySelector('#ctBanner');
  const status = overlay.querySelector('#ctStatus');
  const search = overlay.querySelector('#ctSearch');
  let catalog = { items: [], sourceErrors: [], skipped: 0 };
  let previewName = '';

  const close = () => { endThemePreview(); document.removeEventListener('keydown', onKey); overlay.remove(); };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });
  document.addEventListener('keydown', onKey);
  overlay.querySelector('#ctClose').onclick = close;

  const paintBanner = () => {
    banner.innerHTML = previewName
      ? `<div class="ct-banner"><span>Previewing “${esc(previewName)}” — your theme is untouched until you apply one.</span>
           <button class="btn sm ghost" id="ctStop">Stop preview</button></div>`
      : '';
    banner.querySelector('#ctStop')?.addEventListener('click', () => {
      endThemePreview(); previewName = ''; paintBanner();
    });
  };

  const card = ({ theme, raw, description }) => {
    const saved = isCommunitySaved(theme.id);
    const byline = [theme.author, theme.version ? `v${theme.version}` : ''].filter(Boolean).join(' · ') || 'community theme';
    const actions = saved
      ? `<button class="btn sm" data-act="use">${icon('check', 12)} Use theme</button>
         <button class="btn sm ghost" data-act="remove">${icon('x', 11)} Remove</button>`
      : `<button class="btn sm ghost" data-act="preview">${icon('eye', 12)} Preview</button>
         <button class="btn sm" data-act="download">${icon('download', 12)} Download</button>`;
    const node = el(`<div class="ct-card">
      <span class="ct-sw" style="background:${theme.colors['--bg1']}" aria-hidden="true">
        <span class="sw-bar" style="background:${theme.colors['--accent']}"></span>
        <span class="sw-bar" style="background:${theme.colors['--ok']};width:60%"></span>
        <span class="sw-bar" style="background:${theme.colors['--err']};width:38%"></span>
      </span>
      <div class="ct-meta">
        <div class="ct-name">${esc(theme.name)} ${saved ? '<span class="ct-tag">saved</span>' : ''}
          ${store.prefs.theme === theme.id ? '<span class="ct-tag ok">active</span>' : ''}</div>
        <div class="ct-by">${esc(byline)}</div>
        ${description ? `<div class="ct-desc">${esc(description)}</div>` : ''}
        <div class="ct-actions">${actions}</div>
      </div>
    </div>`);
    node.querySelector('[data-act="use"]')?.addEventListener('click', () => {
      endThemePreview(); previewName = ''; paintBanner();
      if (applyTheme(theme.id)) { setPrefs({ theme: theme.id }); toast('ok', 'Theme applied', theme.name); }
      repaint(); render();
    });
    node.querySelector('[data-act="remove"]')?.addEventListener('click', () => {
      removeSavedCommunityTheme(theme.id);
      if (store.prefs.theme === theme.id) { setPrefs({ theme: 'dark' }); applyTheme('dark'); }
      repaint(); render();
    });
    node.querySelector('[data-act="preview"]')?.addEventListener('click', () => {
      endThemePreview(); previewName = '';
      const res = previewTheme(raw);
      if (res.ok) { previewName = theme.name; paintBanner(); }
    });
    node.querySelector('[data-act="download"]')?.addEventListener('click', async () => {
      const btn = node.querySelector('[data-act="download"]');
      if (btn) { btn.disabled = true; btn.textContent = 'Saving…'; }
      const res = await saveCommunityTheme(raw, { description: description || '' });
      if (!res.ok) {
        toast('err', 'Theme rejected', (res.errors || []).slice(0, 2).join('; '));
        render();
        return;
      }
      toast('ok', 'Theme downloaded', res.theme.name);
      repaint(); render();
    });
    return node;
  };

  const render = () => {
    const q = search.value.trim().toLowerCase();
    const hit = (...vals) => !q || vals.some((v) => (v || '').toLowerCase().includes(q));
    const online = catalog.items.filter((it) => hit(it.theme.name, it.theme.author, it.description));
    // downloaded themes that are NOT in the current catalog (offline registry,
    // removed source, or no sources configured yet) — everything the catalog
    // lists already carries its saved state on its own card
    const onlineIds = new Set(catalog.items.map((it) => it.theme.id));
    const saved = savedCommunityEntries()
      .filter((e) => !onlineIds.has(e.theme.id))
      .filter((e) => hit(e.theme.name, e.theme.author, e.meta.description));
    const parts = [];
    if (online.length) {
      parts.push(el(`<div class="ct-section">Available online (${online.length})</div>`));
      const wrap = el('<div class="ct-list"></div>');
      for (const it of online) wrap.appendChild(card({ theme: it.theme, raw: it.raw, description: it.description }));
      parts.push(wrap);
    }
    if (saved.length) {
      parts.push(el(`<div class="ct-section">Your downloaded themes (${saved.length})</div>`));
      const wrap = el('<div class="ct-list"></div>');
      for (const e of saved) wrap.appendChild(card({ theme: e.theme, raw: e.raw, description: e.meta.description }));
      parts.push(wrap);
    }
    if (!parts.length) {
      const noSources = !listCommunitySources().length;
      parts.push(el(`<div class="empty-state">
        <h3>${noSources ? 'Community sources are not configured yet' : 'No community themes found'}</h3>
        <p>${noSources
          ? 'The community catalog ships empty for now — a public theme registry will be announced later. Once themes are published you will preview and download them right here.'
          : 'Try a different search, or press Refresh to re-fetch the catalog.'}</p>
      </div>`));
    }
    list.replaceChildren(...parts);
    const sources = listCommunitySources();
    const errs = catalog.sourceErrors || [];
    status.innerHTML = sources.length
      ? `${sources.length} source${sources.length > 1 ? 's' : ''} · ${catalog.items.length} theme${catalog.items.length === 1 ? '' : 's'} online · ${savedCommunityEntries().length} downloaded`
        + (errs.length ? `<br><span style="color:var(--warn)">${esc(errs.map((e) => `${e.url}: ${e.message}`).join(' · '))}</span>` : '')
      : 'No community sources configured yet — downloaded themes stay available and fully functional.';
  };

  const refresh = async () => {
    list.replaceChildren(el('<div class="ct-empty">Loading community catalog…</div>'));
    catalog = await fetchCommunityCatalog();
    render();
  };
  overlay.querySelector('#ctRefresh').onclick = refresh;
  search.addEventListener('input', render);
  paintBanner();
  refresh();
}

export { renderSettings as render };
