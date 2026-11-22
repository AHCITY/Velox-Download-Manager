// Vault — successfully completed downloads with real verification state:
// verified (checksum passed), checksummed (hash present, not re-checked),
// unverified (no hash), and file-missing (artifact deleted/moved externally).
import { icon } from '../icons.js';
import { store, fmtBytes, fmtTime, categoryOf, refreshAll } from '../store.js';
import { el, esc, actions } from '../ui.js';

let built = false;
let vaultCat = 'all';
let vaultQuery = '';

export function renderVault(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-vault') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function verifyTag(r) {
  if (r.file_missing) return '<span class="tag warn">file missing</span>';
  if (r.verified) return '<span class="tag ok">verified</span>';
  if (r.sha256 || r.blake3) return '<span class="tag acc">checksummed</span>';
  return '<span class="tag">unverified</span>';
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-vault">
    <div class="screen-head">
      <div><h1>Vault</h1><div class="sub" id="vSub"></div></div>
    </div>
    <div class="list-tools">
      <div class="seg" id="vCats">
        <button data-c="all" class="on">All</button><button data-c="video">Video</button>
        <button data-c="audio">Audio</button><button data-c="image">Images</button>
        <button data-c="archive">Archives</button><button data-c="document">Documents</button>
        <button data-c="software">Software</button><button data-c="other">Other</button>
      </div>
      <div class="tool-search" style="flex:0 0 220px">${icon('search', 13)}<input id="vQ" placeholder="Search vault" aria-label="Search vault"></div>
    </div>
    <div class="note acc" style="margin-bottom:12px">${icon('shieldCheck', 14)}<div><b>Verified</b> means the engine recomputed the checksum and it matched. <b>Checksummed</b> means a hash exists on the record but has not been re-checked since completion — run “Verify file” to check it.</div></div>
    <div class="card" style="padding:6px 16px" id="vList"></div>
  </div>`);
  shell.querySelector('#vCats').onclick = (e) => {
    const b = e.target.closest('[data-c]');
    if (!b) return;
    vaultCat = b.dataset.c;
    shell.querySelectorAll('#vCats button').forEach((x) => x.classList.toggle('on', x === b));
  };
  const q = shell.querySelector('#vQ');
  q.addEventListener('input', () => { vaultQuery = q.value.toLowerCase(); paintShell(shell); });
  return shell;
}

function paintShell(root) {
  let rows = [...store.rows.values()].filter((r) => r.state === 'completed');
  const total = rows.length;
  const missing = rows.filter((r) => r.file_missing).length;
  root.querySelector('#vSub').textContent = `${total} files · ${rows.filter((r) => r.verified).length} verified${missing ? ` · ${missing} missing on disk` : ''}`;
  if (vaultCat !== 'all') rows = rows.filter((r) => categoryOf(r.final_name) === vaultCat);
  if (vaultQuery) rows = rows.filter((r) => `${r.final_name} ${r.dest_dir}`.toLowerCase().includes(vaultQuery));
  rows.sort((a, b) => (b.completed_at || b.updated_at || '').localeCompare(a.completed_at || a.updated_at || ''));

  const list = root.querySelector('#vList');
  if (!rows.length) {
    list.innerHTML = `<div class="empty-state" style="padding:40px 16px">
      <div class="glyph">${icon('vault', 22)}</div>
      <h3>The vault is empty</h3><p>Completed downloads appear here with their verification state and file location.</p>
    </div>`;
    return;
  }
  list.replaceChildren();
  for (const r of rows) {
    const node = el(`<div style="display:grid;grid-template-columns:30px minmax(0,1fr) auto auto;gap:12px;align-items:center;padding:11px 0;border-bottom:1px solid var(--line)">
      <span class="file-ico" style="width:30px;height:30px">${icon({ video: 'fileVideo', audio: 'fileAudio', image: 'fileImage', archive: 'archive', document: 'fileText', software: 'package', code: 'fileCode', other: 'file' }[categoryOf(r.final_name)], 15)}</span>
      <div style="min-width:0">
        <div style="font-weight:600;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(r.final_name)} ${verifyTag(r)}</div>
        <div style="font-size:11px;color:var(--t3);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(r.dest_dir)} · ${esc(fmtTime(r.completed_at))}</div>
      </div>
      <div style="font-size:12px;color:var(--t2);font-variant-numeric:tabular-nums">${fmtBytes(r.total_size)}</div>
      <div style="display:flex;gap:4px">
        <button class="mini-btn" data-a="open" title="Open file" ${r.file_missing ? 'disabled' : ''}>${icon('external', 13)}</button>
        <button class="mini-btn" data-a="reveal" title="Open containing folder">${icon('folderOpen', 13)}</button>
        <button class="mini-btn" data-a="verify" title="Verify file" ${!(r.sha256 || r.blake3) || r.file_missing ? 'disabled' : ''}>${icon('shieldCheck', 13)}</button>
        <button class="mini-btn" data-a="redl" title="Re-download" ${r.file_missing ? '' : 'disabled'}>${icon('refresh', 13)}</button>
      </div>
    </div>`);
    node.querySelector('[data-a="open"]').onclick = () => actions.open(r.id);
    node.querySelector('[data-a="reveal"]').onclick = () => actions.reveal(r.id);
    node.querySelector('[data-a="verify"]').onclick = () => actions.verify(r.id);
    node.querySelector('[data-a="redl"]').onclick = () => actions.redownload(r.id);
    list.appendChild(node);
  }
}
export { renderVault as render };
