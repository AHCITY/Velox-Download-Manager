// Velox shared UI components — context menu, dialogs, download row, details
// drawer. Everything is plain DOM + the design system; no framework.
import { icon } from './icons.js';
import {
  store, fmtBytes, fmtSpeed, fmtEta, fmtTime, pct, categoryOf,
  toast, dismissToast, refreshOne, refreshAll,
  etaState, holdQueueItem, releaseQueueItem, kickQueues, saveQueues,
} from './store.js';
import { engine, EngineError, openFile, revealFolder, copyText, isDesktop, friendlyEngineMessage } from './api.js';

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export function el(html) {
  const t = document.createElement('template');
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
}

export function fileTypeIcon(name, size = 17) {
  const cat = categoryOf(name);
  const glyph = { video: 'fileVideo', audio: 'fileAudio', image: 'fileImage', archive: 'archive', document: 'fileText', software: 'package', code: 'fileCode', other: 'file' }[cat];
  return `<div class="file-ico ${cat === 'document' || cat === 'other' ? (cat === 'other' ? '' : 'doc') : cat}" title="${esc(cat)}">${icon(glyph, size)}</div>`;
}

/** YouTube video id from a media-page URL, else null. */
export function ytThumbId(url) {
  const m = String(url || '').match(/(?:youtube\.com\/(?:watch\?[^#]*v=|shorts\/|embed\/)|youtu\.be\/)([A-Za-z0-9_-]{11})/);
  return m ? m[1] : null;
}

/** Row art: the source-provided thumbnail when the engine captured one (ALL
 *  media sources — YouTube, Vimeo, SoundCloud, …), else the legacy i.ytimg.com
 *  scheme for YouTube URLs whose metadata was unavailable, else the category
 *  icon. The img is attached by paintRow (CSP allows any https image);
 *  on load error it falls back to the icon, so a dead thumb never leaves a hole. */
export function rowArt(r) {
  if (r?.thumbnail) return `<div class="file-ico thumb" data-src="${esc(r.thumbnail)}" title="${esc(r?.final_name || '')}"></div>`;
  const id = ytThumbId(r?.url);
  if (!id) return fileTypeIcon(r?.final_name);
  return `<div class="file-ico thumb" data-thumb="${esc(id)}" title="${esc(r?.final_name || '')}"></div>`;
}

/* ================= toasts (rendered once, patched by store) ================= */
export function renderToasts(root) {
  root.replaceChildren();
  for (const t of store.toasts) {
    const glyph = { ok: 'checkCircle', err: 'alertTriangle', warn: 'alertTriangle', info: 'info' }[t.kind] || 'info';
    const node = el(`<div class="toast ${t.kind}" data-toast="${t.id}" role="status">
      <span class="t-ic">${icon(glyph, 16)}</span>
      <div class="t-msg"><b>${esc(t.title)}</b>${t.msg ? `<span>${esc(t.msg)}</span>` : ''}
        ${t.actions?.length ? `<div class="t-acts">${t.actions.map((a, i) => `<button class="btn sm ghost" data-i="${i}">${esc(a.label)}</button>`).join('')}</div>` : ''}
      </div>
      <button class="t-x" aria-label="Dismiss">${icon('x', 13)}</button>
    </div>`);
    node.querySelector('.t-x').onclick = () => dismissToast(t.id);
    node.querySelectorAll('.t-acts button').forEach((b) => {
      b.onclick = () => { dismissToast(t.id); t.actions[+b.dataset.i].act(); };
    });
    root.appendChild(node);
  }
}

/* ================= context menu ================= */
let ctxEl = null;
export function closeCtx() { if (ctxEl) { ctxEl.remove(); ctxEl = null; } }
export function contextMenu(x, y, items) {
  closeCtx();
  ctxEl = el(`<div class="ctx" role="menu"></div>`);
  for (const it of items) {
    if (it === '-') { ctxEl.appendChild(el('<div class="sep"></div>')); continue; }
    const b = el(`<button class="ci ${it.danger ? 'danger' : ''}" ${it.disabled ? 'disabled' : ''} role="menuitem">
      ${it.icon ? icon(it.icon, 14) : ''}<span>${esc(it.label)}</span>${it.k ? `<span class="k">${esc(it.k)}</span>` : ''}
    </button>`);
    b.onclick = () => { closeCtx(); it.act?.(); };
    ctxEl.appendChild(b);
  }
  document.body.appendChild(ctxEl);
  const r = ctxEl.getBoundingClientRect();
  ctxEl.style.left = Math.min(x, window.innerWidth - r.width - 8) + 'px';
  ctxEl.style.top = Math.min(y, window.innerHeight - r.height - 8) + 'px';
  const first = ctxEl.querySelector('.ci:not(:disabled)');
  if (first) first.focus();
}
document.addEventListener('click', closeCtx, true);
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeCtx(); }, true);
window.addEventListener('blur', closeCtx);

/* ================= modal dialogs ================= */
export function openDialog(html, { onClose } = {}) {
  const ov = el(`<div class="overlay show" role="dialog" aria-modal="true"></div>`);
  const dlg = el(html);
  ov.appendChild(dlg);
  document.body.appendChild(ov);
  const close = () => { ov.remove(); onClose?.(); };
  ov.addEventListener('mousedown', (e) => { if (e.target === ov) close(); });
  const escHandler = (e) => { if (e.key === 'Escape' && document.body.contains(ov)) { e.stopPropagation(); close(); } };
  document.addEventListener('keydown', escHandler, true);
  dlg.querySelectorAll('[data-close]').forEach((b) => b.addEventListener('click', close));
  const focusable = dlg.querySelector('input, select, textarea, button:not([data-close])');
  setTimeout(() => (focusable || dlg).focus?.(), 30);
  return { dlg, close, overlay: ov };
}

export function confirmDialog({ title, message, confirmLabel = 'Confirm', danger = false }) {
  return new Promise((resolve) => {
    // settles exactly once, whichever way the dialog goes away — Cancel, the
    // confirm button, Escape or a click on the backdrop (the last two used to
    // leave the caller's `await` pending forever)
    let settled = false;
    const done = (v) => { if (!settled) { settled = true; resolve(v); } };
    const { dlg, close } = openDialog(`
      <div class="dialog" style="max-width:400px">
        <div class="dialog-head"><h3>${esc(title)}</h3></div>
        <div class="dialog-body"><p style="color:var(--t2);font-size:13px;line-height:1.55">${message}</p></div>
        <div class="dialog-foot">
          <button class="btn" data-close>Cancel</button>
          <button class="btn ${danger ? 'danger' : 'primary'}" data-yes>${esc(confirmLabel)}</button>
        </div>
      </div>`, { onClose: () => done(false) });
    dlg.querySelector('[data-yes]').onclick = () => { done(true); close(); };
  });
}

/* ================= download actions (shared by rows, menus, bulk) ================= */
function friendlyError(e) {
  const code = e instanceof EngineError ? e.code : 'ENGINE';
  const map = {
    INVALID_URL: 'That does not look like a downloadable URL (http/https only).',
    UNSUPPORTED_URL: 'This is a media page — the engine needs the yt-dlp extractor installed and on PATH to resolve it.',
    NOT_FOUND: 'The item no longer exists in the engine database.',
    HTTP: 'The server rejected the request.',
    TIMEOUT: 'The server took too long to answer.',
    CONNECT: 'Could not connect to the server.',
    DNS: 'Could not resolve the server address.',
    IO: 'A local file operation failed.',
    MISSING_FFMPEG: 'FFmpeg is missing — reinstall with the unified installer (it bundles FFmpeg) so video and audio can be combined.',
    GPL_BUILD: 'The installed FFmpeg is a GPL build; install an LGPL build instead.',
    CORRUPT_MEDIA: 'The combined file did not validate as playable media — retry the merge.',
    UNAVAILABLE: 'This needs the desktop app.',
  };
  const friendly = map[code] ? `${map[code]}` : friendlyEngineMessage(e?.message || String(e));
  return code === 'ENGINE' ? friendly : `${friendly} (${code})`;
}
export function reportError(e, context = '') {
  toast('err', context || 'Action failed', friendlyError(e), [], 9000);
}

export const actions = {
  // pause/resume/retry are USER actions on a queue member: record the intent in
  // the queue's `held` list so the orchestrator never overrides it (a merge in
  // progress cannot be paused — the engine refuses it, so it is not offered)
  async pause(id) {
    try {
      await engine.pause(id);
      holdQueueItem(id);
      await refreshOne(id);
    } catch (e) { reportError(e, 'Pause failed'); }
  },
  async resume(id) {
    try {
      await engine.start(id);
      releaseQueueItem(id);
      await refreshOne(id);
    } catch (e) { reportError(e, 'Resume failed'); }
  },
  async retry(id) {
    try {
      await engine.start(id);
      releaseQueueItem(id);
      await refreshOne(id);
      toast('info', 'Retry started', '');
    } catch (e) { reportError(e, 'Retry failed'); }
  },
  async cancel(id) {
    try { await engine.cancel(id); await refreshAll(); } catch (e) { reportError(e, 'Cancel failed'); }
  },
  async remove(id) {
    const r = store.rows.get(id);
    const name = r?.final_name || `#${id}`;
    if (store.prefs.confirmRemove) {
      const ok = await confirmDialog({
        title: 'Remove from list?',
        message: `<b>${esc(name)}</b> will be removed from the download list. The downloaded file (if any) stays on disk.`,
        confirmLabel: 'Remove',
        danger: true,
      });
      if (!ok) return;
    }
    try { await engine.remove(id, false); store.rows.delete(id); store.selection.delete(id); refreshAll(); } catch (e) { reportError(e, 'Remove failed'); }
  },
  async deleteFile(id) {
    const r = store.rows.get(id);
    const name = r?.final_name || `#${id}`;
    const ok = await confirmDialog({
      title: 'Delete downloaded file?',
      message: `The file <b>${esc(name)}</b> will be permanently deleted from disk along with its list entry. This cannot be undone.`,
      confirmLabel: 'Delete file',
      danger: true,
    });
    if (!ok) return;
    try {
      await engine.remove(id, true);
      store.rows.delete(id); store.selection.delete(id);
      refreshAll();
      toast('ok', 'File deleted', name);
    } catch (e) { reportError(e, 'Delete failed'); }
  },
  async open(id) {
    const r = store.rows.get(id);
    if (!r) return;
    const p = `${r.dest_dir}/${r.final_name}`.replace(/\/+/g, '/');
    try { await openFile(p); } catch (e) { reportError(e, 'Open failed'); }
  },
  async reveal(id) {
    const r = store.rows.get(id);
    if (!r) return;
    const p = `${r.dest_dir}/${r.final_name}`.replace(/\/+/g, '/');
    try { await revealFolder(p); } catch (e) {
      if (String(e.message || e).includes('no longer exists')) {
        toast('warn', 'File missing on disk', 'The recorded file no longer exists — it was deleted or moved outside Velox.', [
          { label: 'Re-download', act: () => actions.redownload(id) },
          { label: 'Remove entry', act: () => actions.remove(id) },
        ], 10000);
      } else reportError(e, 'Open folder failed');
    }
  },
  async copyUrl(id) {
    const r = store.rows.get(id);
    if (!r) return;
    const ok = await copyText(r.url);
    toast(ok ? 'ok' : 'warn', ok ? 'URL copied' : 'Could not access clipboard', ok ? '' : r.url);
  },
  async verify(id) {
    try {
      toast('info', 'Verifying…', 'Recomputing checksum');
      const v = await engine.verify(id);
      if (v.matches === true) toast('ok', 'Verification passed', `${v.algo}: ${v.computed.slice(0, 16)}…`);
      else if (v.matches === false) toast('err', 'Verification FAILED', `${v.algo} mismatch — file may be corrupt`, [], 9000);
      else toast('info', 'Checksum stored', `${v.algo}: ${v.computed.slice(0, 16)}… (no expected value to compare)`);
      await refreshOne(id);
    } catch (e) { reportError(e, 'Verify failed'); }
  },
  async redownload(id) {
    const r = store.rows.get(id);
    if (!r) return;
    try {
      const a = await engine.create(r.url, {});
      await engine.start(a.id);
      await refreshAll();
      toast('ok', 'Re-download started', a.final_name || r.final_name);
    } catch (e) { reportError(e, 'Re-download failed'); }
  },
};

// exposed for event-driven actions (store.js toasts)
window.__veloxActions = {
  reveal: (id) => actions.reveal(id),
  retry: (id) => actions.retry(id),
  details: (id) => window.__veloxOpenDetails?.(id),
};

/* ================= download row ================= */
function statusChip(r) {
  const cls = { created: 'queued' }[r.state] || r.state;
  const label = { created: 'queued', retrying: 'retrying', resolving: 'resolving' }[r.state] || r.state;
  return `<span class="chip ${cls}">${label}</span>`;
}

export function downloadRow(r, { selectable = true, compact = false } = {}) {
  const row = el(`<div class="dl-row ${store.selection.has(r.id) ? 'selected' : ''}" data-id="${r.id}" tabindex="0" role="row" aria-label="${esc(r.final_name)}"></div>`);
  paintRow(row, r, { selectable, compact });
  return row;
}

export function paintRow(node, r, { selectable = true, compact = false } = {}) {
  const st = r.state;
  const running = ['downloading', 'retrying'].includes(st);
  const paused = st === 'paused';
  const queued = st === 'queued' || st === 'created';
  const resolving = st === 'resolving';
  const merging = st === 'merging';
  const done = st === 'completed';
  const failed = st === 'failed';
  const cancelled = st === 'cancelled';
  const p = pct(r);
  const bps = running ? (store.bps.get(r.id) || r.bps || 0) : 0;
  const eta = running ? fmtEta(r.received, r.total_size, bps) : null;
  const missing = r.file_missing === true;
  const dim = (v) => `<span class="dim">—</span>`;

  // right-aligned numeric columns (professional list, mono digits)
  const sizeHtml = r.total_size != null
    ? (r.received && r.received < r.total_size && !done
        ? `<b>${fmtBytes(r.received)}</b> <span class="of">/ ${fmtBytes(r.total_size)}</span>`
        : fmtBytes(r.total_size))
    : (r.received ? fmtBytes(r.received) : dim());
  const speedHtml = running && bps ? `<b>${fmtSpeed(bps)}</b>` : (r.bandwidth_limit_bps ? `<span class="of" title="speed limit">≤ ${fmtSpeed(r.bandwidth_limit_bps)}</span>` : dim());
  const etaHtml = eta ? eta.replace(' left', '') : dim();

  const btns = [];
  const mini = (act, ic, title, danger = false) => `<button class="mini-btn ${danger ? 'danger' : ''}" data-act="${act}" title="${title}" aria-label="${title}">${icon(ic, 14)}</button>`;
  if (running) btns.push(mini('pause', 'pause', 'Pause')); // merging is not pausable (Cancel instead)
  if (paused || queued || failed || cancelled || (done && missing)) btns.push(mini(resuming(r) ? 'resume' : 'retry', resuming(r) ? 'play' : 'retry', resuming(r) ? 'Resume' : 'Retry'));
  if (!done && !cancelled) btns.push(mini('cancel', 'x', 'Cancel', true));
  if (done || failed || cancelled) btns.push(mini('open', 'external', 'Open file'));
  if (done || failed || cancelled) btns.push(mini('reveal', 'folderOpen', 'Open containing folder'));
  btns.push(mini('menu', 'more', 'More actions'));
  function resuming(r2) { return paused || queued; }

  // secondary line: domain (when useful) + destination + live connection count
  let domain = '';
  try { domain = new URL(r.url).host || ''; } catch { /* keep empty */ }
  const metaBits = [];
  if (domain) metaBits.push(esc(domain));
  if (running && r.connections_active) {
    const noun = r.mode === 'torrent' ? 'peers' : 'connections';
    metaBits.push(`<b>${r.connections_active}</b> ${noun}`);
  }
  metaBits.push(esc(fmtTime(r.created_at)));
  const domLine = `<div class="row-dom">${metaBits.join('<span class="dotsep">·</span>')}<span class="dst" title="${esc(r.dest_dir)}">${esc(r.dest_dir)}</span></div>`;

  const checkHtml = selectable ? `<div class="row-check"><input type="checkbox" data-check aria-label="Select" ${store.selection.has(r.id) ? 'checked' : ''}></div>` : '<div></div>';

  node.innerHTML = `
    ${checkHtml}
    ${rowArt(r)}
    <div class="row-main">
      <div class="row-name">
        <span class="nm" title="${esc(r.final_name)}">${esc(r.final_name)}</span>
        ${missing ? `<span class="missing">${icon('alertTriangle', 11)} file missing</span>` : ''}
        ${statusChip(r)}
      </div>
      ${compact ? '' : domLine}
      ${failed && (r.error_message || r.error_code) ? `<div class="row-err" title="${esc(r.error_code || '')}: ${esc(r.error_message || '')}">${esc(r.error_message || r.error_code)}</div>`
        : merging ? `<div class="row-media">${icon('merge', 12)} merging streams into the final file…</div>`
        : resolving ? `<div class="row-media">${icon('loader', 12)} resolving media page…</div>`
        : ''}
      <div class="progress-line"><div class="track"><div class="fill ${done ? 'done' : failed ? 'fail' : paused ? 'paused' : merging ? 'merge' : ''}" style="width:${done ? 100 : p == null ? (r.received ? 6 : 0) : p}%"></div></div></div>
    </div>
    <div class="row-stats">
      <div class="rs-size">${sizeHtml}</div>
      <div class="rs-speed">${speedHtml}</div>
      <div class="rs-eta">${etaHtml}</div>
    </div>
    <div class="row-side">
      <div class="pct">${done ? '100%' : p == null ? (r.received ? '…' : '0%') : `${p.toFixed(0)}%`}</div>
      <div class="row-actions">${btns.join('')}</div>
    </div>
  `;
  node.classList.toggle('dim', done || cancelled);
  node.classList.toggle('selected', store.selection.has(r.id));
  node.dataset.compact = compact ? '1' : '0';

  // wiring
  const check = node.querySelector('[data-check]');
  if (check) check.onchange = (e) => {
    if (e.shiftKey && store.lastCheckedId != null) {
      // range select: from last checked to this one within the visible order
      const vis = visibleRows().map((x) => x.id);
      const i1 = vis.indexOf(store.lastCheckedId), i2 = vis.indexOf(r.id);
      if (i1 >= 0 && i2 >= 0) {
        for (const id of vis.slice(Math.min(i1, i2), Math.max(i1, i2) + 1)) store.selection.add(id);
      }
    } else if (check.checked) store.selection.add(r.id); else store.selection.delete(r.id);
    store.lastCheckedId = check.checked ? r.id : null;
    node.classList.toggle('selected', store.selection.has(r.id));
    node.dispatchEvent(new CustomEvent('selchange', { bubbles: true }));
  };
  // YouTube thumbnail: attach after innerHTML so a load error can fall back
  // to the category icon without inline handlers (CSP-safe)
  const thumbBox = node.querySelector('.file-ico.thumb');
  if (thumbBox) {
    const img = new Image();
    img.className = 'row-thumb';
    img.loading = 'lazy';
    img.alt = '';
    img.src = thumbBox.dataset.src || `https://i.ytimg.com/vi/${thumbBox.dataset.thumb}/mqdefault.jpg`;
    img.onerror = () => { thumbBox.classList.remove('thumb'); thumbBox.innerHTML = fileTypeIcon(r.final_name); };
    thumbBox.appendChild(img);
  }
  node.querySelectorAll('[data-act]').forEach((b) => {
    b.onclick = (e) => {
      e.stopPropagation();
      const act = b.dataset.act;
      if (act === 'pause') actions.pause(r.id);
      else if (act === 'resume') actions.resume(r.id);
      else if (act === 'retry') actions.retry(r.id);
      else if (act === 'cancel') actions.cancel(r.id);
      else if (act === 'open') actions.open(r.id);
      else if (act === 'reveal') actions.reveal(r.id);
      else if (act === 'menu') rowMenu(e.clientX, e.clientY, r);
    };
  });
  node.oncontextmenu = (e) => { e.preventDefault(); if (!store.selection.has(r.id)) { store.selection.clear(); store.selection.add(r.id); node.dispatchEvent(new CustomEvent('selchange', { bubbles: true })); } rowMenu(e.clientX, e.clientY, r); };
  // click = select (details are one deliberate action away), double-click =
  // the natural verb: open a finished file, retry a failed one, else details
  node.onclick = (e) => {
    if (e.target.closest('button, input')) return;
    if (!store.selection.has(r.id)) {
      store.selection.clear();
      store.selection.add(r.id);
      node.dispatchEvent(new CustomEvent('selchange', { bubbles: true }));
    }
  };
  node.ondblclick = (e) => {
    if (e.target.closest('button, input')) return;
    if (r.state === 'completed') actions.open(r.id);
    else if (r.state === 'failed' || r.state === 'cancelled') actions.retry(r.id);
    else window.__veloxOpenDetails?.(r.id);
  };
}

export function rowMenu(x, y, r) {
  const st = r.state;
  // Pause only for transfers in flight (a merge cannot be paused; queued rows
  // already offer Resume)
  const pausable = st === 'downloading' || st === 'retrying';
  const done = st === 'completed';
  const items = [];
  if (pausable) items.push({ label: 'Pause', icon: 'pause', act: () => actions.pause(r.id) });
  if (st === 'paused' || st === 'queued' || st === 'created') items.push({ label: 'Resume', icon: 'play', act: () => actions.resume(r.id) });
  if (done && r.file_missing) items.push({ label: 'Retry download', icon: 'retry', act: () => actions.retry(r.id) });
  if (st === 'failed' || st === 'cancelled') items.push({ label: 'Retry', icon: 'retry', act: () => actions.retry(r.id) });
  if (!done && st !== 'cancelled') items.push({ label: 'Cancel', icon: 'x', danger: true, act: () => actions.cancel(r.id) });
  items.push('-');
  items.push({ label: 'Open file', icon: 'external', disabled: !done && st !== 'failed' && st !== 'cancelled', act: () => actions.open(r.id) });
  items.push({ label: 'Open containing folder', icon: 'folderOpen', disabled: !done && st !== 'failed' && st !== 'cancelled', act: () => actions.reveal(r.id) });
  items.push({ label: 'Copy download URL', icon: 'copy', act: () => actions.copyUrl(r.id) });
  items.push({ label: 'View details', icon: 'eye', act: () => window.__veloxOpenDetails?.(r.id) });
  // real queue assignment (queues persist; membership drives the orchestrator)
  if (store.queues.length) {
    const inQ = store.queues.filter((q) => q.itemIds.includes(r.id));
    const open = store.queues.filter((q) => !q.itemIds.includes(r.id));
    if (open.length) {
      items.push('-');
      for (const q of open.slice(0, 4)) {
        items.push({ label: `Add to queue: ${q.name}`, icon: 'queue', act: () => {
          q.itemIds.push(r.id);
          releaseQueueItem(r.id); // no hold left over from an earlier membership
          saveQueues();
          kickQueues(); // a running queue picks the new member up right away
        } });
      }
    }
    if (inQ.length) {
      for (const q of inQ.slice(0, 3)) {
        items.push({ label: `Remove from queue: ${q.name}`, icon: 'minus', act: () => { q.itemIds = q.itemIds.filter((x) => x !== r.id); saveQueues(); } });
      }
    }
  }
  items.push('-');
  items.push({ label: 'Verify file', icon: 'shieldCheck', disabled: !done, act: () => actions.verify(r.id) });
  items.push({ label: 'Re-download', icon: 'refresh', act: () => actions.redownload(r.id) });
  items.push('-');
  items.push({ label: 'Remove from list', icon: 'minus', act: () => actions.remove(r.id) });
  items.push({ label: 'Delete downloaded file', icon: 'trash', danger: true, disabled: !done && !r.file_missing, act: () => actions.deleteFile(r.id) });
  contextMenu(x, y, items);
}

/* ================= details drawer ================= */
export function detailsDrawer() {
  const node = el(`<div class="drawer" id="detailsDrawer" aria-label="Download details">
    <div class="drawer-head">
      ${fileTypeIcon('', 15)}
      <h3 id="dTitle">—</h3>
      <button class="icon-btn" data-close-d aria-label="Close">${icon('x', 15)}</button>
    </div>
    <div class="drawer-body" id="dBody"></div>
  </div>`);
  document.body.appendChild(node);
  node.querySelector('[data-close-d]').onclick = () => node.classList.remove('show');
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') node.classList.remove('show'); });
  return node;
}

export function paintDetails(node, r) {
  const title = node.querySelector('#dTitle');
  const body = node.querySelector('#dBody');
  title.textContent = r.final_name || `#${r.id}`;
  const kv = (k, v, dim = false) => `<div class="k"><span>${k}</span></div><div class="v ${dim ? 'dim' : ''}">${v}</div>`;
  const p = pct(r);
  const segs = (r.segments || []).map((s) => `<div class="seg-cell ${s.completed ? 'full' : s.pos > s.start ? 'done' : ''}" title="segment ${s.idx}: ${fmtBytes(s.pos)} / ${s.end != null ? fmtBytes(s.end - s.start + 1) : 'stream'}"></div>`).join('');
  const errs = (r.recent_errors || []).slice(0, 12).map((l) => `<div class="err-line">${esc(l)}</div>`).join('');
  const shortUrl = (u) => { const s = String(u || ''); return s.length > 46 ? s.slice(0, 46) + '…' : s; };
  const stateCls = { created: 'queued' }[r.state] || r.state;
  const stateLbl = { created: 'queued', retrying: 'retrying' }[r.state] || r.state;
  body.innerHTML = `
    <div class="d-sec">
      <div class="kv">
        ${kv('Status', `<span class="chip ${stateCls}">${stateLbl}</span>`)}
        ${r.file_missing ? kv('File', `<span class="tag warn">${icon('alertTriangle', 11)} missing on disk</span>`) : ''}
        ${kv('Progress', `${fmtBytes(r.received)} ${r.total_size ? 'of ' + fmtBytes(r.total_size) : ''} (${p == null ? 'unknown size' : p.toFixed(1) + '%'})`)}
        ${kv('Speed', fmtSpeed(r.bps_smoothed || store.bps.get(r.id) || 0))}
        ${kv('Saved to', esc(r.dest_dir))}
        ${kv('File name', esc(r.final_name))}
        ${kv('URL', `<span class="url-line"><span class="u" title="${esc(r.url)}">${esc(shortUrl(r.url))}</span><button class="icon-btn sm" data-copyurl title="Copy full URL" aria-label="Copy URL">${icon('copy', 12)}</button></span>`)}
        ${r.state === 'failed' && (r.error_message || r.error_code) ? kv('Problem', esc(friendlyEngineMessage(r.error_message || r.error_code))) : ''}
      </div>
      ${r.state === 'completed' && !r.file_missing ? `<button class="btn sm" style="margin-top:9px" data-dverify>${icon('shieldCheck', 13)} Verify file now</button>` : ''}
    </div>
    <details class="adv">
      <summary>Advanced details</summary>
      <div class="d-sec">
        <div class="kv">
          ${r.resolved_url && r.resolved_url !== r.url ? kv('Resolved to', `<span class="url-line"><span class="u" title="${esc(r.resolved_url)}">${esc(shortUrl(r.resolved_url))}</span></span>`) : ''}
          ${kv('Mode', esc(r.mode), true)}
          ${r.bandwidth_limit_bps ? kv('Speed limit', fmtSpeed(r.bandwidth_limit_bps)) : ''}
          ${kv('Speed (session avg)', r.bps_avg ? fmtSpeed(r.bps_avg) : '—', true)}
          ${kv('Speed (peak)', r.bps_peak ? fmtSpeed(r.bps_peak) : '—', true)}
          ${kv('ETA', (etaState(r).label), true)}
          ${kv('Connections', `${r.connections_active || 0} active · ${r.accept_ranges === 'yes' ? 'range support' : r.accept_ranges === 'no' ? 'no range support' : 'range unknown'}`)}
          ${kv('Resumable', esc(r.resumable), true)}
          ${segs ? `<div class="seg-line" style="margin-top:9px;grid-column:1/-1">${segs}</div>` : ''}
        </div>
      </div>
      <div class="d-sec">
        <h4>Integrity</h4>
        <div class="kv">
          ${r.sha256 ? kv('SHA-256', `<span style="font-family:var(--mono);font-size:11px">${esc(r.sha256)}</span>`) : kv('SHA-256', 'not provided', true)}
          ${r.blake3 ? kv('BLAKE3', `<span style="font-family:var(--mono);font-size:11px">${esc(r.blake3)}</span>`) : ''}
          ${kv('Verified', r.verified ? '<span class="tag ok">passed</span>' : (r.sha256 || r.blake3) ? '<span class="tag acc">has checksum</span>' : '<span class="tag">no checksum</span>')}
          ${r.etag ? kv('ETag', esc(r.etag), true) : ''}
          ${r.last_modified ? kv('Last-Modified', esc(r.last_modified), true) : ''}
        </div>
      </div>
      <div class="d-sec">
        <h4>Timeline</h4>
        <div class="kv">
          ${kv('Added', fmtTime(r.created_at))}
          ${kv('Last activity', fmtTime(r.updated_at))}
          ${r.completed_at ? kv('Completed', fmtTime(r.completed_at)) : ''}
        </div>
      </div>
      <div class="d-sec">
        <h4>Recent errors</h4>
        ${errs || '<div class="kv"><div class="v dim">none recorded</div></div>'}
      </div>
    </details>`;
  const dv = body.querySelector('[data-dverify]');
  if (dv) dv.onclick = () => actions.verify(r.id);
  body.querySelector('[data-copyurl]')?.addEventListener('click', async () => {
    const ok = await copyText(r.url);
    toast(ok ? 'ok' : 'warn', ok ? 'URL copied' : 'Could not access clipboard', ok ? '' : shortUrl(r.url));
  });
}

export { friendlyError };
