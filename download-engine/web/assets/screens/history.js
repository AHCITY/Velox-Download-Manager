// History — the engine's persistent record of finished work: completed, failed
// and crash-recovered cancelled rows. Search, date filter, re-download, remove.
import { icon } from '../icons.js';
import { store, fmtBytes, fmtTime, categoryOf, refreshAll } from '../store.js';
import { el, esc, actions } from '../ui.js';

let built = false;
let histQuery = '';
let histKind = 'all';
let histDate = '';

export function renderHistory(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-history') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-history">
    <div class="screen-head">
      <div><h1>History</h1><div class="sub">Persistent record kept by the engine database — survives restarts</div></div>
    </div>
    <div class="list-tools">
      <div class="tool-search" style="flex:0 0 260px">${icon('search', 13)}
        <input id="hQ" placeholder="Search name, URL or destination" aria-label="Search history">
      </div>
      <select class="btn" id="hKind" style="height:30px">
        <option value="all">All results</option>
        <option value="completed">Completed</option>
        <option value="failed">Failed</option>
        <option value="cancelled">Cancelled</option>
        <option value="paused">Paused / inactive</option>
      </select>
      <input class="btn" id="hDate" type="date" style="height:30px;width:auto;color-scheme:dark" aria-label="Filter by date">
      <button class="btn ghost" id="hClearF">Clear filters</button>
    </div>
    <div class="card" style="padding:6px 16px" id="hList"></div>
  </div>`);
  const q = shell.querySelector('#hQ');
  q.addEventListener('input', () => { histQuery = q.value.toLowerCase(); paintShell(shell); });
  shell.querySelector('#hKind').onchange = (e) => { histKind = e.target.value; paintShell(shell); };
  shell.querySelector('#hDate').onchange = (e) => { histDate = e.target.value; paintShell(shell); };
  shell.querySelector('#hClearF').onclick = () => {
    histQuery = ''; histKind = 'all'; histDate = ''; q.value = '';
    shell.querySelector('#hDate').value = '';
    paintShell(shell);
  };
  return shell;
}

function paintShell(root) {
  let rows = [...store.rows.values()].filter((r) => ['completed', 'failed', 'cancelled', 'paused'].includes(r.state));
  if (histKind !== 'all') rows = rows.filter((r) => r.state === histKind);
  if (histQuery) rows = rows.filter((r) => `${r.final_name} ${r.url} ${r.dest_dir}`.toLowerCase().includes(histQuery));
  if (histDate) {
    rows = rows.filter((r) => {
      const d = (r.completed_at || r.updated_at || r.created_at || '').slice(0, 10);
      return d === histDate;
    });
  }
  rows.sort((a, b) => (b.completed_at || b.updated_at || '').localeCompare(a.completed_at || a.updated_at || ''));

  const list = root.querySelector('#hList');
  if (!rows.length) {
    list.innerHTML = `<div class="empty-state" style="padding:40px 16px">
      <div class="glyph">${icon('history', 22)}</div>
      <h3>No history ${histKind !== 'all' || histQuery || histDate ? 'matches' : 'yet'}</h3>
      <p>${histKind !== 'all' || histQuery || histDate ? 'Try clearing the filters.' : 'Finished, failed and cancelled downloads are recorded here permanently.'}</p>
    </div>`;
    return;
  }
  list.replaceChildren();
  for (const r of rows) {
    const ok = r.state === 'completed';
    const result = ok
      ? (r.file_missing ? '<span class="tag warn">file missing</span>' : r.verified ? '<span class="tag ok">verified</span>' : '<span class="tag">completed</span>')
      : r.state === 'failed' ? '<span class="tag err">failed</span>'
      : r.state === 'cancelled' ? '<span class="tag">cancelled</span>'
      : '<span class="tag">paused</span>';
    const node = el(`<div style="display:grid;grid-template-columns:26px minmax(0,1fr) auto auto;gap:12px;align-items:center;padding:10px 0;border-bottom:1px solid var(--line)">
      <span style="color:${ok ? 'var(--ok)' : r.state === 'failed' ? 'var(--err)' : 'var(--t3)'}">${icon(ok ? 'checkCircle' : r.state === 'failed' ? 'alertTriangle' : 'minus', 15)}</span>
      <div style="min-width:0">
        <div style="font-weight:600;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(r.final_name)} ${result}</div>
        <div style="font-size:11px;color:var(--t3);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(r.dest_dir)}/${esc(r.final_name)} · ${esc(r.url)}</div>
      </div>
      <div style="text-align:right;font-size:11.5px;color:var(--t2);font-variant-numeric:tabular-nums;min-width:70px">
        ${r.total_size ? fmtBytes(r.total_size) : '—'}<div style="color:var(--t3);font-size:10.5px">${esc(fmtTime(r.completed_at || r.updated_at))}</div>
      </div>
      <div style="display:flex;gap:4px">
        <button class="mini-btn" data-a="redl" title="Re-download">${icon('refresh', 13)}</button>
        <button class="mini-btn" data-a="reveal" title="Open containing folder" ${ok && r.file_missing ? 'disabled' : ''}>${icon('folderOpen', 13)}</button>
        <button class="mini-btn" data-a="copy" title="Copy URL">${icon('copy', 13)}</button>
        <button class="mini-btn danger" data-a="rm" title="Remove from history">${icon('trash', 13)}</button>
      </div>
    </div>`);
    node.querySelector('[data-a="redl"]').onclick = () => actions.redownload(r.id);
    node.querySelector('[data-a="reveal"]').onclick = () => actions.reveal(r.id);
    node.querySelector('[data-a="copy"]').onclick = () => actions.copyUrl(r.id);
    node.querySelector('[data-a="rm"]').onclick = () => actions.remove(r.id);
    list.appendChild(node);
  }
}
export { renderHistory as render };
