// Velox app shell — router, toolbar, sidebar, notifications, shortcuts,
// details drawer. Screens are ES modules keyed by route; each screen keeps its
// own persistent shell and repaints its live regions on store changes.
//
// UX 2.0: downloads-first. Downloads is the default and only primary
// destination; the toolbar's main action is a prominent Add Download button
// (no URL-input-first toolbar).
import { icon } from './assets/icons.js';
import {
  store, bootstrap, counts, subscribe, refreshAll, visibleRows, notify, toast, setPrefs,
  PAUSABLE_STATES, ACTIVE_STATES,
} from './assets/store.js';
import {
  $, el, esc, renderToasts, detailsDrawer, paintDetails, actions, contextMenu, openDialog,
} from './assets/ui.js';
import { openAddDialog } from './assets/screens/addDialog.js';

const ROUTES = {
  downloads: { label: 'Downloads', icon: 'list', mod: () => import('./assets/screens/downloads.js') },
  queues: { label: 'Queues', icon: 'queue', mod: () => import('./assets/screens/queues.js') },
  browser: { label: 'Browser', icon: 'globe', mod: () => import('./assets/screens/browser.js') },
  network: { label: 'Network', icon: 'network', mod: () => import('./assets/screens/network.js') },
  history: { label: 'History', icon: 'history', mod: () => import('./assets/screens/history.js') },
  settings: { label: 'Settings', icon: 'settings', mod: () => import('./assets/screens/settings.js') },
};

const GROUPS = [
  ['Downloads', ['downloads']],
  ['Manage', ['queues', 'browser', 'network', 'history']],
  ['Application', ['settings']],
];

let route = 'downloads';
let sideEl, drawer;

function buildSidebar() {
  sideEl = $('#sidebar');
  const nav = sideEl.querySelector('.nav');
  nav.innerHTML = '';
  for (const [glabel, keys] of GROUPS) {
    nav.appendChild(el(`<div class="nav-group">${glabel}</div>`));
    for (const k of keys) {
      const r = ROUTES[k];
      const b = el(`<button class="nav-item" data-route="${k}" title="${esc(r.label)}" aria-label="${esc(r.label)}">
        ${icon(r.icon, 16)}<span class="nav-label">${esc(r.label)}</span>
        ${k === 'downloads' ? '<span class="nav-badge" data-badge="active"></span>' : ''}
        ${k === 'settings' ? '<span class="nav-badge upd" data-badge="update" style="display:none;background:var(--accent);color:#06121f" title="Update available">↑</span>' : ''}
      </button>`);
      b.onclick = () => go(k);
      nav.appendChild(b);
    }
  }
  const foot = el(`<div class="side-foot"><span class="conn-dot"></span><span>connecting…</span></div>`);
  sideEl.appendChild(foot);
  const tog = el(`<button class="icon-btn side-toggle" id="sideToggle" title="Toggle sidebar (Ctrl+B)" aria-label="Toggle sidebar">${icon('panelLeft', 14)}</button>`);
  tog.onclick = toggleSidebar;
  sideEl.appendChild(tog);
}

/** Collapse/expand the sidebar (icon-only rail). State persists via prefs. */
function toggleSidebar() {
  const collapsed = sideEl.classList.toggle('compact');
  setPrefs({ sidebarExpanded: !collapsed });
}

function buildToolbar() {
  const tb = $('#toolbar');
  tb.innerHTML = `
    <button class="btn primary tb-add" id="tbAdd" title="Add download (Ctrl+N)" aria-label="Add download">
      ${icon('plus', 15)} Add Download
    </button>
    <div class="tool-search tb-search">${icon('search', 13)}
      <input id="tbSearch" placeholder="Search downloads — name, URL, folder" aria-label="Search downloads">
    </div>
    <span style="flex:1"></span>
    <span class="agg-pill" id="tbActivePill" title="Downloads currently transferring (downloading, retrying, resolving or merging)">
      Active downloads <b id="tbActive">0</b>
    </span>
    <button class="icon-btn" id="tbBell" title="Recent activity" aria-label="Recent activity">${icon('bell', 15)}</button>
    <button class="icon-btn" id="tbMenu" title="Application menu" aria-label="Application menu">${icon('more', 15)}</button>
  `;
  $('#tbAdd').onclick = () => openAddDialog();
  $('#tbSearch').addEventListener('input', (e) => {
    store.filter.query = e.target.value;
    store.selection.clear();
    if (route !== 'downloads') go('downloads');
    else notify(); // reactive re-render while already on Downloads (the old bug)
  });
  $('#tbBell').onclick = (e) => {
    const items = store.activity.length
      ? store.activity.slice(0, 12).map((a) => ({ label: `${a.ts} — ${a.html.replace(/<[^>]+>/g, '').slice(0, 52)}`, act: () => {} }))
      : [{ label: 'No engine events yet', act: () => {} }];
    contextMenu(e.clientX, e.clientY, items);
  };
  $('#tbMenu').onclick = (e) => {
    contextMenu(e.clientX, e.clientY, [
      { label: 'Add download…', icon: 'plus', k: 'Ctrl N', act: () => openAddDialog() },
      { label: 'Settings', icon: 'settings', act: () => go('settings') },
      { label: 'Check for updates', icon: 'refresh', act: () => { go('settings'); import('./assets/updater.js').then((m) => { m.checkNow(true); }); } },
      '-',
      { label: 'Reload engine data', icon: 'download', act: () => refreshAll().then(() => {}) },
    ]);
  };
}

function go(r) {
  route = r;
  try { history.replaceState(null, '', `#${r}`); } catch { /* webview quirk */ }
  render();
}

let renderToken = 0;
let activeMod = null; // module of the currently rendered screen
async function render() {
  const token = ++renderToken;
  sideEl.querySelectorAll('.nav-item').forEach((b) => b.classList.toggle('active', b.dataset.route === route));
  // QOL: a screen that exports stop* hooks gets them called on nav-away, so
  // live-poll timers (Browser screen's 3s status poll) never outlive the screen
  activeMod?.stopBrowserStatusTimer?.();
  const r = ROUTES[route];
  const mod = await r.mod();
  if (token !== renderToken) return; // a newer navigation won the race
  const target = $('#view');
  target.replaceChildren(el('<div></div>'));
  mod.render(target.firstElementChild);
  activeMod = mod;
}

function paintBadges() {
  const c = counts();
  const badge = sideEl.querySelector('[data-badge="active"]');
  if (badge) {
    const n = c.downloading + c.queued;
    badge.textContent = n ? String(n) : '';
    badge.style.display = n ? '' : 'none';
  }
  const up = sideEl.querySelector('[data-badge="update"]');
  if (up) up.style.display = store.update?.status === 'available' ? '' : 'none';
  // Active Downloads indicator: rows in an ACTIVE engine state only (work in
  // flight). Paused, queued and completed rows are never counted. Replaces the
  // aggregate speed counter (live speed stays on the Network screen and in rows).
  const active = [...store.rows.values()].filter((r) => ACTIVE_STATES.includes(r.state)).length;
  const pill = $('#tbActivePill');
  if (pill) pill.classList.toggle('on', active > 0);
  const tbActive = $('#tbActive');
  if (tbActive && tbActive.textContent !== String(active)) tbActive.textContent = String(active);
}

function paintConnection() {
  const foot = sideEl.querySelector('.side-foot');
  if (!foot) return;
  const dot = foot.querySelector('.conn-dot');
  dot.className = `conn-dot ${store.connected ? 'ok' : 'err'}`;
  foot.lastElementChild.textContent = store.connected ? 'engine connected' : 'engine disconnected — retrying';
}

/* ---------- keyboard shortcuts ---------- */
document.addEventListener('keydown', (e) => {
  const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName) || document.activeElement?.isContentEditable;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'n') { e.preventDefault(); openAddDialog(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') { e.preventDefault(); $('#tbSearch')?.focus(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') { e.preventDefault(); toggleSidebar(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v' && !typing) {
    // paste-to-add: clipboard content (if it looks like a URL) prefills the dialog
    e.preventDefault();
    navigator.clipboard?.readText?.().then((t) => {
      const v = String(t || '').trim();
      openAddDialog(/^https?:\/\//i.test(v) ? v : '');
    }).catch(() => openAddDialog(''));
    return;
  }
  if (typing) return;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
    if (route === 'downloads') {
      e.preventDefault();
      visibleRows().forEach((r) => store.selection.add(r.id));
      notify();
    }
    return;
  }
  if (e.key === ' ' && route === 'downloads' && store.selection.size === 1) {
    e.preventDefault();
    const id = [...store.selection][0];
    const r = store.rows.get(id);
    if (!r) return;
    // Space toggles pause/resume. A merge in progress cannot be paused (the
    // engine refuses it), so it is left alone instead of raising an error.
    if (PAUSABLE_STATES.includes(r.state)) actions.pause(id);
    else if (r.state !== 'merging') actions.retry(id);
    return;
  }
  if ((e.key === 'Delete' || e.key === 'Backspace') && store.selection.size && route === 'downloads') {
    e.preventDefault();
    for (const id of [...store.selection]) actions.remove(id);
    return;
  }
  if (e.key === 'Enter' && route === 'downloads' && store.selection.size === 1) {
    // Enter = the primary verb: open a finished file, otherwise inspect
    const id = [...store.selection][0];
    const r = store.rows.get(id);
    if (r && r.state === 'completed') actions.open(id);
    else window.__veloxOpenDetails?.(id);
  }
});

/* ---------- details drawer ---------- */
window.__veloxOpenDetails = (id) => {
  const r = store.rows.get(id);
  if (!r) return;
  if (!drawer) drawer = detailsDrawer();
  drawer.dataset.id = String(id);
  paintDetails(drawer, r);
  drawer.classList.add('show');
};

/* ---------- browser setup onboarding ----------
 * Version-aware state machine (replaces the single browserSetupSeen boolean,
 * which never re-checked after an app update and suppressed itself even when
 * the native host was broken):
 *   - working extension (connected, or staged + valid host manifest):
 *       remember per app version, never prompt;
 *   - staged but host manifest broken/stale: offer REPAIR once per version;
 *   - nothing staged: offer INSTALL once per version;
 *   - a dismissal is remembered per app version — a new app version rechecks
 *     the real installation state once (never repeatedly interrupts).
 * ------------------------------------------------ */
async function maybeBrowserSetup() {
  const { isDesktop, extensionStatus, extensionSetup } = await import('./assets/api.js');
  if (!isDesktop) return;
  let appVersion = '';
  try { appVersion = String(await window.__TAURI__.app.getVersion() || ''); } catch { /* keep '' */ }
  const status = await extensionStatus();
  if (!status) return;

  const seenFor = (v) => store.prefs.browserSetupStateVersion === v;
  const markFor = (v) => setPrefs({ browserSetupStateVersion: v });
  const dismissedFor = (v) => store.prefs.browserSetupDismissedFor === v;
  const dismissFor = (v) => setPrefs({ browserSetupDismissedFor: v });

  // healthy: staged extension AND a host manifest Chrome/Edge will accept
  // (extension_connected implies both — the host actually talked to the bridge)
  const healthy = status.extension_connected
    || (status.ext_installed && status.host_registered && status.host_manifest_valid);
  if (healthy) { markFor(appVersion); return; }

  // same version already handled (installed, repaired or dismissed): no nag
  if (seenFor(appVersion) || dismissedFor(appVersion)) return;

  const browsers = (status.browsers || []).filter((b) => b.family === 'chromium' && b.exe);
  const repair = status.ext_installed && status.host_registered && status.host_manifest_valid === false;
  const issueText = (status.host_issues || []).join(' ');
  const rows = browsers.map((b) => `
    <div class="setting-row" style="gap:10px">
      <div class="s-txt"><div class="s-name">${esc(b.name)}</div><div class="s-desc">${repair ? 'restages the extension files and re-registers the native host' : 'stages the extension + registers the native host, then opens the extensions page'}</div></div>
      <div class="s-ctl"><button class="btn primary" data-b="${esc(b.id)}">${icon(repair ? 'refresh' : 'download', 13)} ${repair ? 'Repair' : 'Install'}</button></div>
    </div>`).join('');

  const { dlg, close } = openDialog(`
    <div class="dialog" style="max-width:460px">
      <div class="dialog-head"><h3>${repair ? 'Repair your browser connection to Velox' : 'Connect your browser to Velox'}</h3></div>
      <div class="dialog-body">
        ${repair
          ? `<p style="color:var(--t2);font-size:13px;line-height:1.55;margin-bottom:10px">The extension files are staged, but the native messaging host registration is not usable by Chrome/Edge:</p>
             <p class="path" style="color:var(--err);font-size:12px;line-height:1.5;margin-bottom:10px">${esc(issueText || 'host manifest invalid')}</p>`
          : `<p style="color:var(--t2);font-size:13px;line-height:1.55;margin-bottom:10px">Install the official Velox extension to send downloads straight from the web (right-click → <b>Download with Velox</b>). Chromium shows its standard two-click confirmation — nothing is installed silently.</p>`}
        ${rows || '<p style="color:var(--t3);font-size:12px">No Chromium browsers detected — you can set this up later in the Browser screen.</p>'}
        <p style="color:var(--t3);font-size:11.5px;margin-top:10px">Firefox is not supported yet. You can always reach this from the <b>Browser</b> screen.</p>
      </div>
      <div class="dialog-foot">
        <button class="btn" data-later>Set up later</button>
      </div>
    </div>`);

  const settleSeen = async () => markFor(appVersion);
  dlg.querySelectorAll('[data-b]').forEach((btn) => {
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      try {
        const r = await extensionSetup(btn.dataset.b, 'guide');
        toast(r.registered ? 'ok' : 'warn', 'Browser setup',
          r.registered ? `Extension staged and host registered — finish in ${r.browser}: Developer mode → Load unpacked → paste the copied folder path.` : 'Extension staged, but host registration did not verify — see the Browser screen.');
        await settleSeen();
        close();
        location.hash = '#browser';
        go('browser');
      } catch (e) {
        toast('err', 'Browser setup', String(e).replace(/^.*error: */, ''));
        btn.disabled = false;
      }
    });
  });
  dlg.querySelector('[data-later]')?.addEventListener('click', async () => { await dismissFor(appVersion); close(); });
}

/* ---------- boot ---------- */
async function boot() {
  buildSidebar();
  buildToolbar();
  drawer = detailsDrawer();
  renderToasts($('#toasts'));
  await bootstrap();
  import('./assets/updater.js').then((m) => m.initUpdater());
  maybeBrowserSetup();
  document.body.dataset.anim = store.prefs.animations ? 'on' : 'off';
  if (!store.prefs.sidebarExpanded) sideEl.classList.add('compact');
  document.addEventListener('velox:sidebar', (e) => sideEl.classList.toggle('compact', !e.detail));

  // downloads-first: unknown/absent hash lands on the unified list
  const initial = (location.hash || '#downloads').slice(1);
  go(ROUTES[initial] ? initial : 'downloads');

  // QOL fix (found during workspace audit): the hash was read only at boot, so
  // browser back/forward and pasted deep links (#settings) never re-rendered —
  // every screen switch also writes the hash, making it the single source of
  // truth for navigation now.
  window.addEventListener('hashchange', () => {
    const r = (location.hash || '#downloads').slice(1);
    if (ROUTES[r] && r !== route) go(r);
  });

  subscribe(() => {
    paintBadges();
    paintConnection();
    renderToasts($('#toasts'));
    // repaint the active screen's live regions (each screen keeps its shell)
    const node = $('#view').firstElementChild;
    if (node) ROUTES[route].mod().then((m) => m.render(node));
    if (drawer?.classList.contains('show') && drawer.dataset.id) {
      const r = store.rows.get(Number(drawer.dataset.id));
      if (r) paintDetails(drawer, r);
    }
  });
}

boot();
