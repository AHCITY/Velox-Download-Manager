// Downloads — the core screen: filters, search, sort, multi-select, bulk ops,
// context menus, keyed row updates (progress never rebuilds the whole list DOM).
import { icon } from '../icons.js';
import {
  store, visibleRows, counts, setPrefs, refreshAll,
} from '../store.js';
import { engine } from '../api.js';
import { el, esc, downloadRow, paintRow, actions, confirmDialog, reportError } from '../ui.js';

const nodes = new Map(); // id -> row node (keyed updates)

export function renderDownloads(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-downloads') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
    nodes.clear();
  }
  paintShell(root);
  syncList(root);
}

function buildShell() {
  const c = counts();
  const segBtn = (key, label, n) =>
    `<button data-f="${key}" class="${store.filter.state === key ? 'on' : ''}">${label} <span class="n">${n}</span></button>`;
  const shell = el(`<div class="screen" id="screen-downloads">
    <div class="screen-head">
      <div><h1>Downloads</h1><div class="sub" id="dlSub"></div></div>
      <div class="spacer"></div>
      <button class="btn" id="clearDone">${icon('minus', 14)} Clear completed</button>
    </div>
    <div class="list-tools">
      <div class="seg" id="dlSeg">
        ${segBtn('all', 'All', c.all)}
        ${segBtn('downloading', 'Active', c.downloading + c.queued + c.resolving + c.merging)}
        ${segBtn('completed', 'Completed', c.completed)}
        ${segBtn('failed', 'Failed', c.failed)}
        ${segBtn('paused', 'Paused', c.paused)}
      </div>
      <select class="btn" id="dlCat" style="height:30px" aria-label="Category filter">
        <option value="all">All categories</option>
        <option value="video">Video</option><option value="audio">Audio</option>
        <option value="image">Images</option><option value="archive">Archives</option>
        <option value="document">Documents</option><option value="software">Software</option>
        <option value="code">Code</option><option value="other">Other</option>
      </select>
      <select class="btn" id="dlSort" style="height:30px" aria-label="Sort">
        <option value="created_at:desc">Newest first</option>
        <option value="created_at:asc">Oldest first</option>
        <option value="final_name:asc">Name A→Z</option>
        <option value="final_name:desc">Name Z→A</option>
        <option value="total_size:desc">Size largest</option>
        <option value="total_size:asc">Size smallest</option>
        <option value="state:asc">Status</option>
      </select>
      <div class="spacer" style="flex:1"></div>
    </div>
    <div id="selBar"></div>
    <div class="dl-list" id="dlList"></div>
  </div>`);

  shell.querySelector('#dlSeg').onclick = (e) => {
    const b = e.target.closest('[data-f]');
    if (!b) return;
    store.filter.state = b.dataset.f;
    store.selection.clear();
  };
  const cat = shell.querySelector('#dlCat');
  cat.value = store.filter.category;
  cat.onchange = () => { store.filter.category = cat.value; store.selection.clear(); };
  const sort = shell.querySelector('#dlSort');
  sort.value = `${store.sort.key}:${store.sort.dir}`;
  sort.onchange = () => {
    const [k, d] = sort.value.split(':');
    store.sort = { key: k, dir: d };
  };
  shell.querySelector('#clearDone').onclick = async () => {
    const done = [...store.rows.values()].filter((r) => r.state === 'completed');
    if (!done.length) return;
    const withFiles = done.filter((r) => !r.file_missing);
    const ok = await confirmDialog({
      title: 'Clear completed?',
      message: `${done.length} completed ${done.length === 1 ? 'entry' : 'entries'} will be removed from the list.` +
        (withFiles.length ? ` The downloaded ${withFiles.length === 1 ? 'file stays' : 'files stay'} on disk.` : ''),
      confirmLabel: 'Clear list',
      danger: false,
    });
    if (!ok) return;
    for (const r of done) {
      try { await engine.remove(r.id, false); } catch { /* already gone */ }
    }
    await refreshAll();
  };
  shell.addEventListener('selchange', () => paintShell(shell));
  return shell;
}

function paintShell(root) {
  const shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-downloads') return;
  const c = counts();
  root = shell;
  shell.querySelector('#dlSeg').innerHTML = [
    ['all', 'All', c.all],
    ['downloading', 'Active', c.downloading + c.queued + c.resolving + c.merging],
    ['completed', 'Completed', c.completed],
    ['failed', 'Failed', c.failed],
    ['paused', 'Paused', c.paused],
  ].map(([k, l, n]) => `<button data-f="${k}" class="${store.filter.state === k ? 'on' : ''}">${l} <span class="n">${n}</span></button>`).join('');
  const vis = visibleRows();
  root.querySelector('#dlSub').textContent =
    `${c.all} total · ${c.downloading + c.queued} active · ${c.completed} completed` + (store.filter.query ? ` · filtered by “${store.filter.query}”` : '');

  // selection bar
  const selBar = root.querySelector('#selBar');
  const sel = [...store.selection].filter((id) => store.rows.has(id));
  if (sel.length) {
    const rows = sel.map((id) => store.rows.get(id));
    const anyRunning = rows.some((r) => ['downloading', 'retrying', 'merging'].includes(r.state));
    const anyPausedOrQueued = rows.some((r) => ['paused', 'queued', 'created'].includes(r.state));
    const anyFailed = rows.some((r) => ['failed', 'cancelled'].includes(r.state));
    const anyCompleted = rows.some((r) => r.state === 'completed');
    selBar.innerHTML = `<div class="sel-bar">${icon('squareCheck', 14)} ${sel.length} selected
      ${anyRunning ? `<button class="btn sm" data-b="pause">${icon('pause', 12)} Pause</button>` : ''}
      ${anyPausedOrQueued || anyFailed ? `<button class="btn sm" data-b="resume">${icon('play', 12)} Resume</button>` : ''}
      <button class="btn sm" data-b="remove">${icon('minus', 12)} Remove from list</button>
      ${anyCompleted ? `<button class="btn sm danger" data-b="delfile">${icon('trash', 12)} Delete files</button>` : ''}
      <button class="btn sm ghost" data-b="clear">Clear selection</button>
    </div>`;
    selBar.querySelector('[data-b="pause"]')?.addEventListener('click', () => sel.forEach((id) => actions.pause(id)));
    selBar.querySelector('[data-b="resume"]')?.addEventListener('click', () => sel.forEach((id) => actions.retry(id)));
    selBar.querySelector('[data-b="remove"]')?.addEventListener('click', () => sel.forEach((id) => actions.remove(id)));
    selBar.querySelector('[data-b="delfile"]')?.addEventListener('click', () => sel.forEach((id) => actions.deleteFile(id)));
    selBar.querySelector('[data-b="clear"]')?.addEventListener('click', () => { store.selection.clear(); });
  } else selBar.innerHTML = '';
}

function syncList(root) {
  const vis = visibleRows();
  const list = root.querySelector('#dlList');
  const compact = store.prefs.density === 'compact';
  const seen = new Set();

  if (!vis.length) {
    list.replaceChildren(emptyNode());
    nodes.clear();
    return;
  }
  for (const r of vis) {
    seen.add(r.id);
    let node = nodes.get(r.id);
    if (!node) {
      node = downloadRow(r, { compact });
      nodes.set(r.id, node);
      list.appendChild(node);
    } else {
      paintRow(node, r, { compact });
    }
  }
  for (const [id, node] of nodes) {
    if (!seen.has(id)) { node.remove(); nodes.delete(id); }
  }
}

function emptyNode() {
  const f = store.filter.state;
  const map = {
    all: ['download', 'No downloads yet', 'Paste a URL in the toolbar above, or press Ctrl+N, to add your first download.'],
    downloading: ['zap', 'Nothing active', 'Downloads that are currently transferring will appear here.'],
    completed: ['checkCircle', 'No completed downloads', 'Finished downloads land here — use the Vault for file management.'],
    failed: ['alertTriangle', 'No failed downloads', 'When a download cannot finish, it shows up here with the exact reason.'],
    paused: ['pause', 'Nothing paused', 'Paused downloads keep their progress and resume exactly where they left off.'],
  };
  const [glyph, title, text] = map[f] || map.all;
  return el(`<div class="empty-state">
    <div class="glyph">${icon(glyph, 24)}</div>
    <h3>${esc(title)}</h3><p>${esc(text)}</p>
  </div>`);
}
export { renderDownloads as render };
