// Velox app shell — router, toolbar, sidebar, notifications, shortcuts,
// details drawer. Screens are ES modules keyed by route; each screen keeps its
// own persistent shell and repaints its live regions on store changes.
//
// UX 2.0: downloads-first. Downloads is the default and only primary
// destination; the toolbar's main action is a prominent Add Download button
// (no URL-input-first toolbar).
import { icon } from './assets/icons.js';
import {
  store, bootstrap, counts, subscribe, refreshAll, visibleRows, fmtSpeed, notify, toast,
} from './assets/store.js';
import {
  $, el, esc, renderToasts, detailsDrawer, paintDetails, actions, contextMenu,
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
    <span class="agg-pill" title="Combined speed of active downloads">
      <span class="live" id="tbLive"></span><b id="tbAgg">0 B/s</b>
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
async function render() {
  const token = ++renderToken;
  sideEl.querySelectorAll('.nav-item').forEach((b) => b.classList.toggle('active', b.dataset.route === route));
  const r = ROUTES[route];
  const mod = await r.mod();
  if (token !== renderToken) return; // a newer navigation won the race
  const target = $('#view');
  target.replaceChildren(el('<div></div>'));
  mod.render(target.firstElementChild);
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
  let sum = 0;
  for (const r of store.rows.values()) if (r.state === 'downloading') sum += store.bps.get(r.id) || 0;
  const agg = $('#tbAgg'), live = $('#tbLive');
  if (agg) agg.textContent = fmtSpeed(sum);
  if (live) live.classList.toggle('on', sum > 0);
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
    if (r) (['downloading', 'retrying', 'merging'].includes(r.state) ? actions.pause(id) : actions.retry(id));
    return;
  }
  if ((e.key === 'Delete' || e.key === 'Backspace') && store.selection.size && route === 'downloads') {
    e.preventDefault();
    for (const id of [...store.selection]) actions.remove(id);
    return;
  }
  if (e.key === 'Enter' && route === 'downloads' && store.selection.size === 1) {
    window.__veloxOpenDetails?.([...store.selection][0]);
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

/* ---------- first-launch browser setup ----------
 * Browser-ecosystem turn: on the FIRST desktop launch, offer the one-click
 * extension install (Browser screen owns the full flow). Never shown again
 * once dismissed; the screen stays available under Browser.
 * ------------------------------------------------ */
async function maybeBrowserSetupDialog() {
  const { isDesktop, extensionStatus, extensionSetup } = await import('./assets/api.js');
  if (!isDesktop) return;
  if (store.prefs.browserSetupSeen) return;
  const status = await extensionStatus();
  if (status?.extension_connected || status?.ext_installed) {
    store.prefs.browserSetupSeen = true;
    const { saveSettings } = await import('./assets/api.js');
    saveSettings(store.prefs);
    return; // already set up — no nag
  }
  const { openDialog } = await import('./assets/ui.js');
  const { el } = await import('./assets/ui.js');
  const browsers = (status?.browsers || []).filter((b) => b.family === 'chromium' && b.exe);
  const rows = browsers.map((b) => `
    <div class="setting-row" style="gap:10px">
      <div class="s-txt"><div class="s-name">${esc(b.name)}</div><div class="s-desc">stages the extension + registers the native host, then opens the extensions page</div></div>
      <div class="s-ctl"><button class="btn primary" data-b="${esc(b.id)}">${icon('download', 13)} Install</button></div>
    </div>`).join('');
  const { dlg, close } = openDialog(`
    <div class="dialog" style="max-width:460px">
      <div class="dialog-head"><h3>Connect your browser to Velox</h3></div>
      <div class="dialog-body">
        <p style="color:var(--t2);font-size:13px;line-height:1.55;margin-bottom:10px">Install the official Velox extension to send downloads straight from the web (right-click → <b>Download with Velox</b>). Chromium shows its standard two-click confirmation — nothing is installed silently.</p>
        ${rows || '<p style="color:var(--t3);font-size:12px">No Chromium browsers detected — you can set this up later in the Browser screen.</p>'}
        <p style="color:var(--t3);font-size:11.5px;margin-top:10px">Firefox is not supported yet. You can always reach this from the <b>Browser</b> screen.</p>
      </div>
      <div class="dialog-foot">
        <button class="btn" data-later>Set up later</button>
      </div>
    </div>`);
  const markSeen = async () => { store.prefs.browserSetupSeen = true; await saveSettingsSafely(); };
  async function saveSettingsSafely() {
    const { saveSettings } = await import('./assets/api.js');
    try { await saveSettings(store.prefs); } catch { /* non-fatal */ }
  }
  dlg.querySelectorAll('[data-b]').forEach((btn) => {
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      try {
        const r = await extensionSetup(btn.dataset.b, 'guide');
        toast(r.registered ? 'ok' : 'warn', 'Browser setup',
          r.registered ? `Extension staged and host registered — finish in ${r.browser}: Developer mode → Load unpacked → paste the copied folder path.` : 'Extension staged, but host registration failed — see the Browser screen.');
        await markSeen();
        close();
        location.hash = '#browser';
        go('browser');
      } catch (e) {
        toast('err', 'Browser setup', String(e).replace(/^.*error: */, ''));
        btn.disabled = false;
      }
    });
  });
  dlg.querySelector('[data-later]')?.addEventListener('click', async () => { await markSeen(); close(); });
}

/* ---------- boot ---------- */
async function boot() {
  buildSidebar();
  buildToolbar();
  drawer = detailsDrawer();
  renderToasts($('#toasts'));
  await bootstrap();
  import('./assets/updater.js').then((m) => m.initUpdater());
  maybeBrowserSetupDialog();
  document.body.dataset.anim = store.prefs.animations ? 'on' : 'off';
  if (!store.prefs.sidebarExpanded) sideEl.classList.add('compact');
  document.addEventListener('velox:sidebar', (e) => sideEl.classList.toggle('compact', !e.detail));

  // downloads-first: unknown/absent hash lands on the unified list
  const initial = (location.hash || '#downloads').slice(1);
  go(ROUTES[initial] ? initial : 'downloads');

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
