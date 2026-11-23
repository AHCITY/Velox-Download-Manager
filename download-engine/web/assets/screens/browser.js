// Browser integration — extension installer + live status. The desktop app
// stages the bundled extension, registers the native messaging host, and opens
// the selected Chromium browser's extensions page; this screen drives that
// setup and shows the honest live state (connected = the host has actually
// talked to the engine's bridge recently).
import { icon } from '../icons.js';
import { toast } from '../store.js';
import { el, esc } from '../ui.js';
import { extensionSetup, extensionStatus, isDesktop } from '../api.js';

let statusTimer = null;
let lastStatus = null;

export function renderBrowser(root) {
  if (!isDesktop) {
    root.replaceChildren();
    root.appendChild(el(`<div class="screen" id="screen-browser">
      <div class="screen-head"><div><h1>Browser</h1><div class="sub">Browser extension setup</div></div></div>
      <div class="note warn">${icon('alertTriangle', 14)}<div>Extension installation runs in the Velox desktop app. Browser preview mode cannot stage or register the native host.</div></div>
    </div>`));
    return;
  }
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-browser') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
  refreshStatus(shell);
  if (!statusTimer) statusTimer = setInterval(() => refreshStatus(shell), 3000);
}

export function stopBrowserStatusTimer() {
  if (statusTimer) { clearInterval(statusTimer); statusTimer = null; }
}

function buildShell() {
  return el(`<div class="screen" id="screen-browser">
    <div class="screen-head">
      <div><h1>Browser</h1><div class="sub">Install the Velox extension and see live integration status</div></div>
    </div>
    <div class="grid-2">
      <div>
        <div class="card" style="padding:16px 18px;margin-bottom:14px">
          <h1 style="font-size:14px;margin-bottom:10px">Integration status</h1>
          <div class="setting-row"><div class="s-txt"><div class="s-name">Extension connected</div><div class="s-desc">A browser extension has talked to Velox through the native host.</div></div>
            <div class="s-ctl" data-st="connected"><span class="tag">checking…</span></div></div>
          <div class="setting-row"><div class="s-txt"><div class="s-name">Native host registered</div><div class="s-desc">velox-browser-host is registered with at least one Chromium browser.</div></div>
            <div class="s-ctl" data-st="host"><span class="tag">checking…</span></div></div>
          <div class="setting-row"><div class="s-txt"><div class="s-name">Extension staged</div><div class="s-desc">Unpacked extension copied to the stable per-user folder.</div></div>
            <div class="s-ctl" data-st="staged"><span class="tag">checking…</span></div></div>
          <div class="setting-row"><div class="s-txt"><div class="s-name">yt-dlp extractor</div><div class="s-desc">Resolves media pages (YouTube etc.) into stream URLs.</div></div>
            <div class="s-ctl" data-st="ytdlp"><span class="tag">checking…</span></div></div>
          <div class="s-desc" data-st="extpath" style="margin-top:6px;font-family:var(--mono);font-size:11px;color:var(--t3)"></div>
        </div>
        <div class="card" style="padding:16px 18px">
          <h1 style="font-size:14px;margin-bottom:10px">What the extension gives you</h1>
          <div class="act-feed">
            <div class="act-item"><span class="ts">1.</span><span class="tx"><b>Download with Velox</b> in the right-click menu on any link, page or selected URL.</span></div>
            <div class="act-item"><span class="ts">2.</span><span class="tx">Toolbar popup: live speed, active count, recent downloads, paste-and-add.</span></div>
            <div class="act-item"><span class="ts">3.</span><span class="tx">Optional download interception: Velox asks before taking over suitable downloads (your choice; never silent).</span></div>
          </div>
        </div>
      </div>
      <div>
        <div class="card" style="padding:16px 18px;margin-bottom:14px">
          <h1 style="font-size:14px;margin-bottom:10px">Install in your browser</h1>
          <div data-st="browsers" class="act-feed"><div class="empty" style="color:var(--t3)">detecting browsers…</div></div>
          <div class="note warn" style="margin-top:10px">${icon('alertTriangle', 14)}<div>Chromium's security model does not allow fully silent installs — the button does everything automatable (staging, host registration, opens the extensions page) and the page shows the last two clicks: enable <b>Developer mode</b> → <b>Load unpacked</b> → pick the shown folder (also copied to your clipboard). <b>Quick start</b> loads the extension immediately for the current browser session — fully quit the browser first if it is running. On <b>Chrome 137+</b> Google removed the <b>--load-extension</b> flag from branded Chrome, so <b>Quick start</b> is silently ignored there; use the guided steps.</div></div>
        </div>
        <div class="card" style="padding:16px 18px">
          <h1 style="font-size:14px;margin-bottom:10px">Firefox</h1>
          <div class="s-desc">Firefox forbids programmatic extension installs and its MV3 event pages differ from Chromium service workers — honest status: <span class="tag warn">not supported yet</span>. A manual temporary load is possible via <b>about:debugging</b> → Load Temporary Add-on, pointing at the staged folder shown on the left.</div>
        </div>
      </div>
    </div>
  </div>`);
}

function tagFor(kind, text) {
  const cls = { ok: 'tag ok', warn: 'tag warn', err: 'tag err', info: 'tag' }[kind] || 'tag';
  return `<span class="${cls}">${esc(text)}</span>`;
}

function paintShell(shell) {
  if (!lastStatus) return;
  const s = lastStatus;
  const set = (k, html) => { const n = shell.querySelector(`[data-st="${k}"]`); if (n) n.innerHTML = html; };
  set('connected', s.extension_connected
    ? tagFor('ok', 'connected')
    : (s.last_bridge_secs_ago != null ? tagFor('warn', `last seen ${s.last_bridge_secs_ago}s ago`) : tagFor('warn', 'not connected')));
  set('host', s.host_registered ? tagFor('ok', 'registered') : tagFor('warn', 'not registered'));
  set('staged', s.ext_installed ? tagFor('ok', 'ready') : tagFor('warn', 'not staged'));
  set('ytdlp', s.ytdlp === 'bundled' ? tagFor('ok', 'bundled') : s.ytdlp === 'path' ? tagFor('ok', 'on PATH') : tagFor('warn', 'not found'));
  set('extpath', s.ext_installed ? `staged: ${s.ext_dir}` : '');

  const host = shell.querySelector('[data-st="browsers"]');
  if (!s.browsers.length) {
    host.innerHTML = '<div class="empty" style="color:var(--t3)">No browsers detected.</div>';
    return;
  }
  host.innerHTML = '';
  for (const b of s.browsers) {
    const row = el(`<div class="setting-row" style="gap:10px">
      <div class="s-txt"><div class="s-name">${esc(b.name)}</div><div class="s-desc">${b.exe ? esc(b.exe) : 'not found'}</div></div>
      <div class="s-ctl" style="display:flex;gap:8px">
        ${b.family === 'chromium' && b.exe ? `
          <button class="btn primary" data-inst="${esc(b.id)}">Install extension</button>
          <button class="btn ghost" data-quick="${esc(b.id)}" title="Temporary session load (--load-extension)">Quick start</button>`
        : tagFor('warn', b.family === 'firefox' ? 'manual only' : 'not installed')}
      </div>
    </div>`);
    host.appendChild(row);
  }
  host.querySelectorAll('[data-inst]').forEach((btn) => {
    btn.addEventListener('click', () => runSetup(shell, btn.dataset.inst, 'guide'));
  });
  host.querySelectorAll('[data-quick]').forEach((btn) => {
    btn.addEventListener('click', () => runSetup(shell, btn.dataset.quick, 'quick'));
  });
}

async function runSetup(shell, id, mode) {
  const r = await extensionSetup(id, mode).catch((e) => ({ error: String(e) }));
  if (r.error) {
    toast('err', 'Browser setup', String(r.error));
    return;
  }
  const bits = [r.registered ? 'native host registered' : 'native host NOT registered'];
  bits.push(r.launched ? `${r.browser} opened` : 'browser did not launch');
  toast(r.registered ? 'ok' : 'warn', 'Browser setup', `Extension staged → ${bits.join(' · ')}`);
  if (r.extension_dir) {
    try { await navigator.clipboard.writeText(r.extension_dir); } catch { /* clipboard may be denied */ }
  }
  await refreshStatus(shell);
}

async function refreshStatus(shell) {
  if (!shell?.isConnected) return; // screen navigated away
  const s = await extensionStatus();
  if (!s) return;
  lastStatus = s;
  paintShell(shell);
}

export { renderBrowser as render };
