// Home dashboard — real numbers from the engine, live active rows, aggregate
// network state, recent activity. Nothing invented.
import { icon } from '../icons.js';
import {
  store, counts, visibleRows, fmtBytes, fmtSpeed, fmtEta, fmtTime, pct, categoryOf, netSnapshot,
} from '../store.js';
import { el, esc, downloadRow, paintRow } from '../ui.js';
import { isDesktop } from '../api.js';

let built = false;
let activeNodes = new Map();

export function renderHome(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-home') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
    activeNodes.clear();
  }
  paintShell(shell);
  syncActive(shell);
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-home">
    <div class="screen-head">
      <div><h1>Home</h1><div class="sub" id="hmSub"></div></div>
      <div class="spacer"></div>
      <span class="agg-pill"><span class="live" id="hmLive"></span><span class="lbl">network</span><b id="hmAgg">0 B/s</b></span>
    </div>
    <div class="stat-row" id="hmStats"></div>
    <div class="grid-2">
      <section>
        <div class="screen-head" style="margin-bottom:10px"><h1 style="font-size:14px">Active downloads</h1></div>
        <div class="dl-list" id="hmActive"></div>
      </section>
      <section>
        <div class="screen-head" style="margin-bottom:10px"><h1 style="font-size:14px">Activity</h1></div>
        <div class="card" style="padding:6px 14px" id="hmFeed"><div class="act-feed" id="hmFeedList"></div></div>
        <div class="card chart-card" style="margin-top:14px">
          <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px">
            <h1 style="font-size:13px">Throughput</h1>
            <span style="flex:1"></span>
            <span style="font-size:11px;color:var(--t3)">last 60s · peak <b id="hmPeak">—</b></span>
          </div>
          <canvas class="net-chart" id="hmChart"></canvas>
        </div>
      </section>
    </div>
  </div>`);
  return shell;
}

function statTile(key, label, cls = '') {
  return `<div class="stat" data-go="${key}"><div class="v ${cls}" id="st-${key}">—</div><div class="l">${label}</div></div>`;
}

function paintShell(root) {
  const c = counts();
  const net = netSnapshot();
  root.querySelector('#hmSub').textContent = store.engine ? `engine v${store.engine.version} · API v${store.engine.api_version}` : '';
  root.querySelector('#hmStats').innerHTML =
    statTile('all', 'Total downloads') +
    statTile('downloading', 'Active now', net.active ? 'acc' : '') +
    statTile('queued', 'Queued', c.queued ? 'warn' : '') +
    statTile('completed', 'Completed', c.completed ? 'ok' : '') +
    statTile('failed', 'Failed', c.failed ? 'err' : '');
  for (const [k, v] of Object.entries(c)) {
    const elN = root.querySelector(`#st-${k}`);
    if (elN) elN.textContent = String(v);
  }
  const live = root.querySelector('#hmLive');
  if (live) live.classList.toggle('on', net.active > 0);
  root.querySelector('#hmAgg').textContent = fmtSpeed(net.aggBps);
  const feed = root.querySelector('#hmFeedList');
  const items = store.activity.slice(0, 12);
  feed.innerHTML = items.length
    ? items.map((a) => `<div class="act-item"><span class="ts">${esc(a.ts)}</span><span class="tx">${a.html}</span></div>`).join('')
    : `<div class="act-item"><span class="tx">Events from the engine appear here — add a download to see activity.</span></div>`;
  const peak = root.querySelector('#hmPeak');
  if (peak) peak.textContent = fmtBytes(store.net.peak) + '/s';
  drawChart(root.querySelector('#hmChart'));
}

function drawChart(canvas) {
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  if (!w) return;
  if (canvas.width !== w * dpr) { canvas.width = w * dpr; canvas.height = h * dpr; }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const s = store.net.samples.slice(-60);
  if (!s.length) return;
  const max = Math.max(1, ...s.map((p) => p.bps));
  const y = (v) => h - 6 - (v / max) * (h - 14);
  // grid
  ctx.strokeStyle = 'rgba(255,255,255,.05)';
  ctx.beginPath();
  for (let i = 1; i < 4; i++) { ctx.moveTo(0, (h / 4) * i); ctx.lineTo(w, (h / 4) * i); }
  ctx.stroke();
  // area + line
  const x = (i) => (i / Math.max(1, s.length - 1)) * (w - 2) + 1;
  const grad = ctx.createLinearGradient(0, 0, 0, h);
  grad.addColorStop(0, 'rgba(77,159,255,.30)');
  grad.addColorStop(1, 'rgba(77,159,255,0)');
  ctx.beginPath();
  ctx.moveTo(x(0), h - 6);
  s.forEach((p, i) => ctx.lineTo(x(i), y(p.bps)));
  ctx.lineTo(x(s.length - 1), h - 6);
  ctx.closePath();
  ctx.fillStyle = grad;
  ctx.fill();
  ctx.beginPath();
  s.forEach((p, i) => (i ? ctx.lineTo(x(i), y(p.bps)) : ctx.moveTo(x(i), y(p.bps))));
  ctx.strokeStyle = '#4d9fff';
  ctx.lineWidth = 1.6;
  ctx.stroke();
}

function syncActive(root) {
  const box = root.querySelector('#hmActive');
  const rows = visibleRows()
    .filter((r) => ['downloading', 'queued', 'created', 'retrying', 'resolving', 'merging', 'paused'].includes(r.state))
    .sort((a, b) => (b.state === 'downloading' ? 1 : 0) - (a.state === 'downloading' ? 1 : 0) || b.id - a.id)
    .slice(0, 6);
  const seen = new Set();
  if (!rows.length) {
    box.replaceChildren(el(`<div class="empty-state" style="padding:30px 16px">
      <div class="glyph">${icon('zap', 20)}</div>
      <h3>Nothing downloading</h3><p>Active downloads with live progress will show up here.</p>
    </div>`));
    activeNodes.clear();
    return;
  }
  for (const r of rows) {
    seen.add(r.id);
    let node = activeNodes.get(r.id);
    if (!node) { node = downloadRow(r, { selectable: false }); activeNodes.set(r.id, node); box.appendChild(node); }
    else paintRow(node, r, { selectable: false });
  }
  for (const [id, n] of activeNodes) if (!seen.has(id)) { n.remove(); activeNodes.delete(id); }
}
export { renderHome as render };
