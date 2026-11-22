// Network — live aggregate throughput (event-sampled), connections, per-download
// speeds, and the engine's real concurrency/limit configuration. Limits that the
// engine does not expose are stated honestly rather than faked.
import { icon } from '../icons.js';
import { store, fmtBytes, fmtSpeed, netSnapshot } from '../store.js';
// recent engine events surface here since Home was retired (UX 2.0)
import { el, esc } from '../ui.js';

let built = false;

export function renderNetwork(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-network') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-network">
    <div class="screen-head">
      <div><h1>Network</h1><div class="sub" id="nwSub"></div></div>
      <div class="spacer"></div>
      <span class="agg-pill"><span class="live" id="nwLive"></span><span class="lbl">aggregate</span><b id="nwAgg">0 B/s</b></span>
    </div>
    <div class="card chart-card" style="margin-bottom:14px">
      <div style="display:flex;align-items:center;gap:10px;margin-bottom:8px">
        <h1 style="font-size:13px">Throughput</h1><span style="flex:1"></span>
        <span style="font-size:11px;color:var(--t3)">peak <b id="nwPeak">—</b> · samples every second</span>
      </div>
      <canvas class="net-chart" id="nwChart" style="height:170px"></canvas>
    </div>
    <div class="stat-row" style="grid-template-columns:repeat(auto-fit,minmax(130px,1fr))">
      <div class="stat"><div class="v acc" id="nwActive">—</div><div class="l">Active downloads</div></div>
      <div class="stat"><div class="v" id="nwConns">—</div><div class="l">Open connections</div></div>
      <div class="stat"><div class="v" id="nwMaxConc">—</div><div class="l">Max concurrent (engine)</div></div>
      <div class="stat"><div class="v" id="nwMaxHost">—</div><div class="l">Connections / host</div></div>
    </div>
    <div class="grid-2">
      <div class="card" style="padding:6px 14px">
        <div class="screen-head" style="margin:8px 0 4px"><h1 style="font-size:13.5px">Per-download speed</h1></div>
        <div class="act-feed" id="nwRows"></div>
      </div>
      <div>
        <div class="card" style="padding:14px 16px;margin-bottom:14px">
          <h1 style="font-size:13.5px;margin-bottom:10px">Speed limits</h1>
          <div class="kv" style="grid-template-columns:150px 1fr">
            <div class="k">Global limit</div><div class="v dim">not supported by the engine — set per download in the Add dialog (token-bucket enforced)</div>
            <div class="k">Per-download</div><div class="v" id="nwLimits">—</div>
          </div>
        </div>
        <div class="note acc">${icon('info', 14)}<div>The engine enforces per-download limits with a token bucket set at creation time. Changing a running download's limit is not yet available; pause and re-add with a new limit if you need one.</div></div>
      </div>
    </div>
    <div class="card" style="padding:6px 14px;margin-top:14px">
      <div class="screen-head" style="margin:8px 0 4px"><h1 style="font-size:13.5px">Recent engine events</h1></div>
      <div class="act-feed" id="nwFeed"></div>
    </div>
  </div>`);
  return shell;
}

function paintShell(root) {
  const net = netSnapshot();
  root.querySelector('#nwAgg').textContent = fmtSpeed(net.aggBps);
  root.querySelector('#nwLive').classList.toggle('on', net.active > 0);
  root.querySelector('#nwSub').textContent = 'Live engine traffic — updates once per second from progress events';
  root.querySelector('#nwActive').textContent = String(net.active);
  root.querySelector('#nwConns').textContent = String(net.conns);
  root.querySelector('#nwMaxConc').textContent = store.engine?.max_concurrent_downloads ?? '—';
  root.querySelector('#nwMaxHost').textContent = store.engine?.max_connections_per_host ?? '—';
  const peak = root.querySelector('#nwPeak');
  if (peak) peak.textContent = fmtBytes(store.net.peak) + '/s';
  drawChart(root.querySelector('#nwChart'));

  const rowsEl = root.querySelector('#nwRows');
  const act = [...store.rows.values()]
    .filter((r) => r.state === 'downloading')
    .map((r) => ({ r, bps: store.bps.get(r.id) || 0 }))
    .sort((a, b) => b.bps - a.bps);
  rowsEl.innerHTML = act.length
    ? act.map(({ r, bps }) => {
      const share = net.aggBps ? Math.round((100 * bps) / net.aggBps) : 0;
      return `<div class="act-item">
        <span class="ts">${icon('zap', 12)}</span>
        <span class="tx" style="flex:1"><b>${esc(r.final_name)}</b></span>
        <span style="width:130px"><span class="track" style="display:inline-block;width:80px"><span class="fill" style="display:block;width:${share}%"></span></span></span>
        <span style="font-variant-numeric:tabular-nums;min-width:86px;text-align:right"><b>${fmtSpeed(bps)}</b></span>
      </div>`;
    }).join('')
    : '<div class="act-item"><span class="tx">No active transfers.</span></div>';

  const limits = [...store.rows.values()].filter((r) => r.bandwidth_limit_bps);
  root.querySelector('#nwLimits').textContent = limits.length
    ? limits.map((r) => `${r.final_name}: ≤ ${fmtSpeed(r.bandwidth_limit_bps)}`).join(' · ')
    : 'none set';

  const feed = root.querySelector('#nwFeed');
  if (feed) {
    const items = store.activity.slice(0, 14);
    feed.innerHTML = items.length
      ? items.map((a) => `<div class="act-item"><span class="ts">${esc(a.ts)}</span><span class="tx">${a.html}</span></div>`).join('')
      : '<div class="act-item"><span class="tx">Engine events appear here as downloads run.</span></div>';
  }
}

function drawChart(canvas) {
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  if (!w) return;
  if (canvas.width !== Math.floor(w * dpr)) { canvas.width = w * dpr; canvas.height = h * dpr; }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const s = store.net.samples.slice(-120);
  if (!s.length) return;
  const max = Math.max(1, ...s.map((p) => p.bps));
  const y = (v) => h - 8 - (v / max) * (h - 18);
  const x = (i) => (i / Math.max(1, s.length - 1)) * (w - 2) + 1;
  ctx.strokeStyle = 'rgba(255,255,255,.05)';
  ctx.beginPath();
  for (let i = 1; i < 4; i++) { ctx.moveTo(0, (h / 4) * i); ctx.lineTo(w, (h / 4) * i); }
  ctx.stroke();
  const grad = ctx.createLinearGradient(0, 0, 0, h);
  grad.addColorStop(0, 'rgba(77,159,255,.32)');
  grad.addColorStop(1, 'rgba(77,159,255,0)');
  ctx.beginPath();
  ctx.moveTo(x(0), h - 8);
  s.forEach((p, i) => ctx.lineTo(x(i), y(p.bps)));
  ctx.lineTo(x(s.length - 1), h - 8);
  ctx.closePath();
  ctx.fillStyle = grad;
  ctx.fill();
  ctx.beginPath();
  s.forEach((p, i) => (i ? ctx.lineTo(x(i), y(p.bps)) : ctx.moveTo(x(i), y(p.bps))));
  ctx.strokeStyle = '#4d9fff';
  ctx.lineWidth = 1.7;
  ctx.stroke();
}
export { renderNetwork as render };
