// Queues — persisted download queues orchestrated through real engine
// start/pause calls. Honest constraint: the engine has no scheduler contract,
// so queues advance only while the app is open (no hidden timers that pretend
// otherwise).
import { icon } from '../icons.js';
import { store, saveQueues, refreshAll, toast, RUNNING_STATES, kickQueues, releaseQueueItem } from '../store.js';
import { engine } from '../api.js';
import { el, esc, confirmDialog, reportError } from '../ui.js';

let built = false;

export function renderQueues(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-queues') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-queues">
    <div class="screen-head">
      <div><h1>Queues</h1><div class="sub">Group downloads and control how many run at once</div></div>
      <div class="spacer"></div>
      <button class="btn primary" id="qNew">${icon('plus', 14)} New queue</button>
    </div>
    <div class="note acc" style="margin-bottom:14px">
      ${icon('info', 14)}<div>Queues orchestrate real engine operations (start/pause) while Velox is open. The engine itself
      does not yet persist a scheduler — a stopped queue resumes exactly where it was when you reopen the app.</div>
    </div>
    <div id="qList"></div>
    <div class="screen-head" style="margin:22px 0 10px"><h1 style="font-size:14px">Assign downloads</h1></div>
    <div class="note" style="margin-bottom:10px">${icon('info', 14)}<div>Pick any download below and assign it to a queue. Queue membership is stored locally and survives restarts.</div></div>
    <div class="card" style="padding:8px 14px" id="qAssignPool"></div>
  </div>`);
  shell.querySelector('#qNew').onclick = () => createQueue();
  return shell;
}

function paintShell(root) {
  const list = root.querySelector('#qList');
  list.replaceChildren();
  if (!store.queues.length) {
    list.appendChild(el(`<div class="empty-state">
      <div class="glyph">${icon('queue', 24)}</div>
      <h3>No queues yet</h3><p>Create a queue, set how many of its downloads may run at once, and assign downloads below.</p>
    </div>`));
  }
  for (const q of store.queues) {
    const items = q.itemIds.map((id) => store.rows.get(id)).filter(Boolean);
    const running = items.filter((r) => ['downloading', 'retrying', 'merging', 'resolving'].includes(r.state)).length;
    const waiting = items.filter((r) => ['queued', 'paused', 'created'].includes(r.state)).length;
    const completed = items.filter((r) => r.state === 'completed').length;
    const node = el(`<div class="q-item">
      <span style="color:${q.active ? 'var(--ok)' : 'var(--t3)'}">${icon(q.active ? 'play' : 'pause', 15)}</span>
      <div class="q-name">${esc(q.name)}${q.active ? ' <span class="tag ok">running</span>' : ' <span class="tag">paused</span>'}</div>
      <div class="q-meta">${running} running · ${waiting} waiting · ${completed}/${items.length} done · ≤${q.maxConcurrent} parallel</div>
      <button class="btn sm" data-a="toggle">${q.active ? 'Pause queue' : 'Start queue'}</button>
      <button class="icon-btn" data-a="settings" aria-label="Queue settings">${icon('sliders', 14)}</button>
      <button class="icon-btn" data-a="del" aria-label="Delete queue" style="color:var(--t2)">${icon('trash', 14)}</button>
    </div>`);
    node.querySelector('[data-a="toggle"]').onclick = () => {
      if (q.active) {
        q.active = false;
        saveQueues();
        for (const id of q.itemIds) {
          const r = store.rows.get(id);
          if (r && ['downloading', 'retrying'].includes(r.state)) engine.pause(id).catch(() => {});
        }
        refreshAll();
      } else {
        import('../store.js').then((m) => m.startQueueItem(q.id));
      }
    };
    node.querySelector('[data-a="settings"]').onclick = () => queueSettings(q);
    node.querySelector('[data-a="del"]').onclick = async () => {
      const ok = await confirmDialog({ title: 'Delete queue?', message: `<b>${esc(q.name)}</b> will be removed. Downloads themselves are untouched.`, confirmLabel: 'Delete', danger: true });
      if (!ok) return;
      store.queues = store.queues.filter((x) => x.id !== q.id);
      saveQueues();
      paintShell(root);
    };
    list.appendChild(node);
  }

  // assignment pool: every row not in any queue
  const assigned = new Set(store.queues.flatMap((q) => q.itemIds));
  const pool = [...store.rows.values()].filter((r) => !assigned.has(r.id) && !['cancelled'].includes(r.state));
  const poolEl = root.querySelector('#qAssignPool');
  if (!pool.length) {
    poolEl.innerHTML = '<div style="color:var(--t3);font-size:12.5px;padding:6px 0">Every download is already assigned, or the list is empty.</div>';
  } else {
    poolEl.replaceChildren();
    for (const r of pool.slice(0, 30)) {
      const row = el(`<div style="display:flex;align-items:center;gap:10px;padding:7px 0;border-bottom:1px solid var(--line)">
        <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12.5px">${esc(r.final_name)}</span>
        <span class="chip ${r.state}">${r.state}</span>
        <select class="btn sm" style="height:26px"><option value="">Assign to…</option>
          ${store.queues.map((q) => `<option value="${q.id}">${esc(q.name)}</option>`).join('')}
        </select>
      </div>`);
      row.querySelector('select').onchange = (e) => {
        const qid = e.target.value;
        if (!qid) return;
        const q = store.queues.find((x) => x.id === qid);
        if (q && !q.itemIds.includes(r.id)) {
          q.itemIds.push(r.id);
          // membership means "run as part of this queue": a hold left over from an
          // earlier membership is cleared (a later hand-pause sets it again)
          releaseQueueItem(r.id);
          saveQueues();
          kickQueues(); // a running queue picks the new member up right away
          toast('ok', 'Assigned', `${r.final_name} → ${q.name}`);
          paintShell(root);
        }
      };
      poolEl.appendChild(row);
    }
  }
}

function createQueue() {
  import('../ui.js').then(({ openDialog }) => {
    const { dlg, close } = openDialog(`<div class="dialog" style="max-width:400px">
      <div class="dialog-head"><h3>New queue</h3></div>
      <div class="dialog-body">
        <div class="field"><label>Name</label><input id="nqName" type="text" placeholder="e.g. Nightly bulk downloads" value="New queue"></div>
        <div class="field"><label>Max parallel downloads</label><select id="nqMax">
          <option>1</option><option selected>2</option><option>3</option><option>4</option><option>6</option><option>8</option>
        </select></div>
      </div>
      <div class="dialog-foot"><button class="btn" data-close>Cancel</button><button class="btn primary" data-ok>Create</button></div>
    </div>`);
    dlg.querySelector('[data-ok]').onclick = () => {
      const name = dlg.querySelector('#nqName').value.trim() || 'Queue';
      const maxConcurrent = parseInt(dlg.querySelector('#nqMax').value, 10);
      store.queues.push({ id: Date.now(), name, maxConcurrent, active: false, itemIds: [] });
      saveQueues();
      close();
      toast('ok', 'Queue created', name);
    };
    dlg.querySelector('#nqName').select();
  });
}

function queueSettings(q) {
  import('../ui.js').then(({ openDialog }) => {
    const { dlg, close } = openDialog(`<div class="dialog" style="max-width:420px">
      <div class="dialog-head"><h3>Queue — ${esc(q.name)}</h3></div>
      <div class="dialog-body">
        <div class="field"><label>Name</label><input id="qsName" type="text" value="${esc(q.name)}"></div>
        <div class="field"><label>Max parallel downloads</label><select id="qsMax">
          ${[1, 2, 3, 4, 6, 8].map((n) => `<option ${n === q.maxConcurrent ? 'selected' : ''}>${n}</option>`).join('')}
        </select></div>
        <div class="d-sec" style="margin-top:10px"><h4>Items (${q.itemIds.length})</h4><div id="qsItems"></div></div>
      </div>
      <div class="dialog-foot"><button class="btn danger" data-del>Delete queue</button><span style="flex:1"></span><button class="btn" data-close>Close</button></div>
    </div>`);
    const items = dlg.querySelector('#qsItems');
    items.replaceChildren();
    q.itemIds.map((id) => store.rows.get(id)).filter(Boolean).forEach((r) => {
      const rowEl = el(`<div style="display:flex;align-items:center;gap:9px;padding:6px 0;border-bottom:1px solid var(--line)">
        <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12.5px">${esc(r.final_name)}</span>
        <span class="chip ${r.state}">${r.state}</span>
        <button class="icon-btn" style="width:24px;height:24px" title="Remove from queue">${icon('x', 12)}</button>
      </div>`);
      rowEl.querySelector('button').onclick = () => {
        q.itemIds = q.itemIds.filter((x) => x !== r.id);
        saveQueues();
        rowEl.remove();
      };
      items.appendChild(rowEl);
    });
    if (!q.itemIds.length) items.innerHTML = '<div style="color:var(--t3);font-size:12px">No items assigned.</div>';
    dlg.querySelector('[data-del]').onclick = () => {
      store.queues = store.queues.filter((x) => x.id !== q.id);
      saveQueues();
      close();
    };
    dlg.querySelector('#qsName').onchange = (e) => { q.name = e.target.value.trim() || q.name; saveQueues(); };
    dlg.querySelector('#qsMax').onchange = (e) => { q.maxConcurrent = parseInt(e.target.value, 10); saveQueues(); };
  });
}
export { renderQueues as render };
