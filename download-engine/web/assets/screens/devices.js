// Devices — honest capability screen. The engine has no network-device, Wi-Fi
// or tethering integrations; this page shows what the engine actually knows
// (storage locations, DB facts) and marks the rest unavailable.
import { icon } from '../icons.js';
import { store, counts } from '../store.js';
import { el, esc } from '../ui.js';

let built = false;

export function renderDevices(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-devices') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-devices">
    <div class="screen-head">
      <div><h1>Devices</h1><div class="sub">Local machine and engine storage facts</div></div>
    </div>
    <div class="grid-2">
      <div class="card" style="padding:16px 18px">
        <h1 style="font-size:14px;margin-bottom:10px">This machine</h1>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Engine data directory</div><div class="s-desc">SQLite database, checkpoints and profiles.</div></div></div>
        <div class="kv" style="padding:4px 0 8px"><div class="v" style="grid-column:1/3;font-family:var(--mono);font-size:11px" id="dvData">—</div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Default download directory</div><div class="s-desc">Where downloads land unless overridden per download.</div></div></div>
        <div class="kv" style="padding:4px 0 8px"><div class="v" style="grid-column:1/3;font-family:var(--mono);font-size:11px" id="dvOut">—</div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Downloads on record</div><div class="s-desc">Rows in the engine database right now.</div></div>
          <div class="s-ctl"><span class="tag acc" id="dvCount">—</span></div></div>
      </div>
      <div class="card" style="padding:16px 18px">
        <h1 style="font-size:14px;margin-bottom:10px">Network devices &amp; tethering</h1>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Wi-Fi / Ethernet interface info</div><div class="s-desc">Interface selection and diagnostics.</div></div>
          <div class="s-ctl"><span class="tag warn">not implemented</span></div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">USB tethering</div><div class="s-desc">Route Velox traffic over a tethered phone connection.</div></div>
          <div class="s-ctl"><span class="tag warn">not implemented</span></div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Paired devices</div><div class="s-desc">Send downloads between machines running Velox.</div></div>
          <div class="s-ctl"><span class="tag warn">not implemented</span></div></div>
        <div class="note" style="margin-top:10px">${icon('info', 14)}<div>No device integrations exist in the engine today. This screen will expose them through a real backend contract when they arrive — nothing here is simulated.</div></div>
      </div>
    </div>
  </div>`);
  return shell;
}

function paintShell(root) {
  root.querySelector('#dvData').textContent = store.engine?.data_dir || '—';
  root.querySelector('#dvOut').textContent = store.engine?.default_out_dir || '—';
  root.querySelector('#dvCount').textContent = String(counts().all);
}
export { renderDevices as render };
