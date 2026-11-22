// Velox app shell — router, toolbar, sidebar, notifications, shortcuts,
// details drawer. Screens are ES modules keyed by route; each screen keeps its
// own persistent shell and repaints its live regions on store changes.
import { icon } from './assets/icons.js';
import {
  store, bootstrap, counts, subscribe, refreshAll, visibleRows, fmtSpeed,
} from './assets/store.js';
import {
  $, el, esc, renderToasts, detailsDrawer, paintDetails, actions,
} from './assets/ui.js';
import { openAddDialog } from './assets/screens/addDialog.js';

const ROUTES = {
  home: { label: 'Home', icon: 'home', mod: () => import('./assets/screens/home.js') },
  downloads: { label: 'Downloads', icon: 'list', mod: () => import('./assets/screens/downloads.js') },
  queues: { label: 'Queues', icon: 'queue', mod: () => import('./assets/screens/queues.js') },
  browser: { label: 'Browser', icon: 'globe', mod: () => import('./assets/screens/browser.js') },
  network: { label: 'Network', icon: 'network', mod: () => import('./assets/screens/network.js') },
  devices: { label: 'Devices', icon: 'devices', mod: () => import('./assets/screens/devices.js') },
  history: { label: 'History', icon: 'history', mod: () => import('./assets/screens/history.js') },
  vault: { label: 'Vault', icon: 'vault', mod: () => import('./assets/screens/vault.js') },
  settings: { label: 'Settings', icon: 'settings', mod: () => import('./assets/screens/settings.js') },
};

let route = 'home';
let sideEl, drawer;

function buildSidebar() {
  sideEl = $('#sidebar');
  const nav = sideEl.querySelector('.nav');
  const groups = [
    ['Overview', ['home', 'downloads']],
    ['Manage', ['queues', 'browser', 'network', 'devices', 'history', 'vault', 'settings']],
  ];
  nav.innerHTML = '';
  for (const [glabel, keys] of groups) {
    nav.appendChild(el(`<div class="nav-group">${glabel}</div>`));
    for (const k of keys) {
      const r = ROUTES[k];
      const b = el(`<button class="nav-item" data-route="${k}" title="${esc(r.label)}" aria-label="${esc(r.label)}">
        ${icon(r.icon, 16)}<span class="nav-label">${esc(r.label)}</span>
        ${k === 'downloads' ? '<span class="nav-badge" data-badge="active"></span>' : ''}
        ${k === 'history' ? '<span class="nav-badge" data-badge="history"></span>' : ''}
        ${k === 'vault' ? '<span class="nav-badge" data-badge="vault"></span>' : ''}
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
    <div class="url-wrap">
      ${icon('link', 14)}
      <input id="tbUrl" type="url" placeholder="Paste a URL and press Enter — http(s) or a media page" spellcheck="false" autocomplete="off" aria-label="Download URL">
      <button class="icon-btn" id="tbAdd" title="Add download (Ctrl+N)" aria-label="Add download">${icon('plus', 15)}</button>
    </div>
    <div class="tool-search">${icon('search', 13)}<input id="tbSearch" placeholder="Search downloads" aria-label="Search downloads"></div>
    <span style="flex:1"></span>
    <span class="agg-pill"><span class="live" id="tbLive"></span><b id="tbAgg">0 B/s</b></span>
    <button class="icon-btn" id="tbBell" title="Activity" aria-label="Activity">${icon('bell', 15)}</button>
  `;
  $('#tbAdd').onclick = () => openAddDialog();
  $('#tbUrl').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      const v = $('#tbUrl').value.trim();
      if (!v) return;
      openAddDialog(v);
      $('#tbUrl').value = '';
    }
  });
  $('#tbSearch').addEventListener('input', (e) => {
    store.filter.query = e.target.value;
    if (route !== 'downloads') go('downloads');
  });
  $('#tbBell').onclick = (e) => {
    import('./assets/ui.js').then(({ contextMenu }) => {
      const items = store.activity.length
        ? store.activity.slice(0, 12).map((a) => ({ label: `${a.ts} — ${a.html.replace(/<[^>]+>/g, '').slice(0, 52)}`, act: () => {} }))
        : [{ label: 'No engine events yet', act: () => {} }];
      contextMenu(e.clientX, e.clientY, items);
    });
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
  const h = sideEl.querySelector('[data-badge="history"]');
  if (h) {
    const n = c.completed + c.failed;
    h.textContent = n ? String(n) : '';
    h.style.display = n ? '' : 'none';
  }
  const v = sideEl.querySelector('[data-badge="vault"]');
  if (v) {
    v.textContent = c.completed ? String(c.completed) : '';
    v.style.display = c.completed ? '' : 'none';
  }
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
  if (typing) return;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
    if (route === 'downloads') {
      e.preventDefault();
      visibleRows().forEach((r) => store.selection.add(r.id));
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

/* ---------- boot ---------- */
async function boot() {
  buildSidebar();
  buildToolbar();
  drawer = detailsDrawer();
  renderToasts($('#toasts'));
  await bootstrap();
  document.body.dataset.anim = store.prefs.animations ? 'on' : 'off';
  if (!store.prefs.sidebarExpanded) sideEl.classList.add('compact');
  document.addEventListener('velox:sidebar', (e) => sideEl.classList.toggle('compact', !e.detail));

  const initial = (location.hash || '#home').slice(1);
  go(ROUTES[initial] ? initial : 'home');

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
