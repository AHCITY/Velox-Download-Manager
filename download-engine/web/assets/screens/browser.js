// Browser integration — honest status. Velox ships NO browser extension today:
// this screen documents what exists (URL paste capture, yt-dlp media-page
// resolution) and what does not, with no fake toggles.
import { icon } from '../icons.js';
import { store, toast } from '../store.js';
import { el, esc } from '../ui.js';
import { copyText } from '../api.js';

let built = false;

export function renderBrowser(root) {
  let shell = root.firstElementChild;
  if (!shell || shell.id !== 'screen-browser') {
    root.replaceChildren();
    shell = buildShell();
    root.appendChild(shell);
  }
  paintShell(shell);
}

function buildShell() {
  const shell = el(`<div class="screen" id="screen-browser">
    <div class="screen-head">
      <div><h1>Browser</h1><div class="sub">Browser integration status and supported capture paths</div></div>
    </div>
    <div class="grid-2">
      <div class="card" style="padding:16px 18px">
        <h1 style="font-size:14px;margin-bottom:10px">Integration status</h1>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Browser extension</div><div class="s-desc">Intercepts downloads from Chrome/Firefox/Edge and hands them to Velox.</div></div>
          <div class="s-ctl"><span class="tag warn">not implemented</span></div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Clipboard monitoring</div><div class="s-desc">Watches the clipboard for URLs in the background.</div></div>
          <div class="s-ctl"><span class="tag warn">not implemented</span></div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Manual capture</div><div class="s-desc">Paste a URL into the toolbar (Ctrl+V) or press Ctrl+N for the full Add dialog.</div></div>
          <div class="s-ctl"><span class="tag ok">available</span></div></div>
        <div class="setting-row"><div class="s-txt"><div class="s-name">Media-page resolution</div><div class="s-desc">Optional yt-dlp extractor resolves media pages (YouTube etc.) into stream URLs.</div></div>
          <div class="s-ctl"><span class="tag ${store.engine?.merge_capable ? 'ok' : 'warn'}">${store.engine?.merge_capable ? 'ffmpeg ready' : 'yt-dlp required'}</span></div></div>
      </div>
      <div>
        <div class="card" style="padding:16px 18px;margin-bottom:14px">
          <h1 style="font-size:14px;margin-bottom:10px">How to capture downloads today</h1>
          <div class="act-feed">
            <div class="act-item"><span class="ts">1.</span><span class="tx">Copy the direct file URL from your browser (right-click → Copy link address).</span></div>
            <div class="act-item"><span class="ts">2.</span><span class="tx">Paste it into the Velox toolbar — the resource is inspected before the download starts.</span></div>
            <div class="act-item"><span class="ts">3.</span><span class="tx">For media pages (YouTube, …) install <b>yt-dlp</b> and make sure it is on PATH; Velox resolves the page automatically when you paste it.</span></div>
          </div>
        </div>
        <div class="note warn">${icon('alertTriangle', 14)}<div>The extension and clipboard monitor are planned, not present. No setting on this screen pretends to enable them — when a real backend arrives, this page will expose it.</div></div>
      </div>
    </div>
  </div>`);
  return shell;
}

function paintShell(root) {
  const tag = root.querySelector('.tag');
  if (tag && store.engine?.merge_capable != null) { /* refreshed on next paint */ }
}
export { renderBrowser as render };
