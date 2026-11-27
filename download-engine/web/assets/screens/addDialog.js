// Add Download dialog — inspects the URL against the real engine before
// starting, and every field maps to an actual DownloadConfig the engine accepts.
import { icon } from '../icons.js';
import { store, fmtBytes, categoryOf, refreshAll, toast } from '../store.js';
import { engine, defaultDownloadDir, EngineError } from '../api.js';
import { openDialog, el, esc, reportError, actions } from '../ui.js';

let lastDest = null;

/** Magnet URIs and .torrent files are torrent sources: they skip the HTTP
 *  inspect preview and are created directly (the swarm reports name/size). */
function isTorrentSource(url) {
  const u = String(url || '').trim().toLowerCase();
  return u.startsWith('magnet:') || u.endsWith('.torrent');
}

export function openAddDialog(prefillUrl = '') {
  // destination precedence: this session's last choice → the persisted default
  // directory (Settings → General) → the engine's startup dir
  const dest = lastDest || store.prefs?.defaultDir || store.engine?.default_out_dir || '';
  const { dlg, close } = openDialog(`
    <div class="dialog" style="max-width:560px" id="addDlg">
      <div class="dialog-head">
        ${icon('download', 16)}
        <h3>Add download</h3>
        <button class="icon-btn" data-close aria-label="Close">${icon('x', 15)}</button>
      </div>
      <div class="dialog-body">
        <div class="field">
          <label for="adUrl">Download URL</label>
          <input id="adUrl" type="url" placeholder="https://example.com/file.zip" spellcheck="false" autocomplete="off" value="${esc(prefillUrl)}">
          <div class="fe" id="adUrlErr" style="display:none"></div>
        </div>
        <div class="preview" id="adPrev" style="display:none"></div>
        <div class="fgrid">
          <div class="field">
            <label for="adName">File name</label>
            <input id="adName" type="text" placeholder="from server or URL">
            <div class="hint">Leave empty to use the server's name</div>
          </div>
          <div class="field">
            <label for="adCat">Category</label>
            <input id="adCat" type="text" disabled placeholder="auto-detected">
          </div>
        </div>
        <div class="field">
          <label for="adDest">Destination folder</label>
          <input id="adDest" type="text" value="${esc(dest)}" spellcheck="false">
        </div>
        <div class="fgrid">
          <div class="field">
            <label for="adConns">Connections</label>
            <select id="adConns">
              <option value="" ${!store.prefs?.defaultConnections ? 'selected' : ''}>Engine default</option>
              ${[1, 2, 4, 8, 16].map((n) => `<option value="${n}" ${String(store.prefs?.defaultConnections) === String(n) ? 'selected' : ''}>${n === 1 ? '1 (single stream)' : n}</option>`).join('')}
            </select>
          </div>
          <div class="field">
            <label for="adLimit">Speed limit</label>
            <select id="adLimit">
              <option value="">Unlimited</option>
              <option value="262144">256 KiB/s</option>
              <option value="1048576">1 MiB/s</option>
              <option value="4194304">4 MiB/s</option>
              <option value="10485760">10 MiB/s</option>
            </select>
            <div class="hint">Per-download token bucket (engine-enforced)</div>
          </div>
        </div>
        <div class="field">
          <label for="adHash">Expected checksum (optional)</label>
          <input id="adHash" type="text" placeholder="sha256 or blake3 hex value" spellcheck="false" style="font-family:var(--mono);font-size:11.5px">
          <div class="hint">The engine refuses to claim completion when the hash mismatches</div>
        </div>
        <div class="collapse" id="adAdv">
          <button type="button" id="adAdvBtn">${icon('chevronDown', 13)} Advanced options</button>
          <div class="body">
            <div class="field">
              <label for="adRef">Referer</label>
              <input id="adRef" type="text" placeholder="https://example.com/page">
            </div>
            <div class="field">
              <label for="adCookie">Cookies</label>
              <input id="adCookie" type="text" placeholder="name=value; name2=value2">
              <div class="hint">Sent to the first request's origin only; RFC 6265 store updates follow redirects</div>
            </div>
            <div class="field" style="display:flex;align-items:center;gap:12px">
              <label class="toggle"><input type="checkbox" id="adOverwrite" ${store.prefs?.defaultOverwrite ? 'checked' : ''}><span class="tk"></span></label>
              <div><div style="font-weight:600;font-size:12.5px">Overwrite existing file</div>
              <div class="hint">Otherwise a numbered name (file(1).ext) is chosen</div></div>
            </div>
            <div class="field" style="display:flex;align-items:center;gap:12px">
              <label class="toggle"><input type="checkbox" id="adAllowHtml"><span class="tk"></span></label>
              <div><div style="font-weight:600;font-size:12.5px">Allow HTML documents</div>
              <div class="hint">Explicitly accept a .html/.htm body as the download</div></div>
            </div>
            <div class="field" style="display:flex;align-items:center;gap:12px">
              <label class="toggle"><input type="checkbox" id="adInsecure"><span class="tk"></span></label>
              <div><div style="font-weight:600;font-size:12.5px;color:var(--warn)">Skip TLS verification</div>
              <div class="hint">Insecure — only for servers with broken certificates you trust</div></div>
            </div>
          </div>
        </div>
      </div>
      <div class="dialog-foot">
        <button class="btn" data-close>Cancel</button>
        <button class="btn primary" id="adGo">${icon('download', 14)} Download</button>
      </div>
    </div>`);

  const $ = (s) => dlg.querySelector(s);
  const urlIn = $('#adUrl'), nameIn = $('#adName'), catIn = $('#adCat'),
    prev = $('#adPrev'), urlErr = $('#adUrlErr'), goBtn = $('#adGo');

  // default destination: desktop exposes the OS download dir via Tauri
  if (!lastDest) {
    defaultDownloadDir().then((d) => {
      if (d && !$('#adDest').value) $('#adDest').value = d;
    });
  }

  $('#adAdvBtn').onclick = () => $('#adAdv').classList.toggle('open');

  // ---- media format selection (YouTube): real qualities from the engine ----
  // The picker only appears after the engine's media.formats probe returns;
  // every quality shown comes from that response — nothing is fabricated.
  const mediaChoice = {
    active: false,
    format: store.prefs.mediaFormat === 'mp3' ? 'mp3' : 'mp4',
    height: null, fps: null,
  };
  let mediaSummary = null; // last probe result; format toggles re-render, not re-probe
  const fmtRow = el(`<div class="field" id="adMediaFmt" style="display:none">
    <label>Media format</label>
    <div class="seg" id="adFmtSeg">
      <button type="button" data-f="mp4">MP4 · video + audio</button>
      <button type="button" data-f="mp3">MP3 · audio only</button>
    </div>
    <div id="adQualityWrap" style="margin-top:8px"></div>
    <div class="hint" id="adFmtNote"></div>
  </div>`);
  {
    const urlField = $('#adUrl').closest('.field');
    if (urlField && urlField.nextSibling) urlField.parentNode.insertBefore(fmtRow, urlField.nextSibling);
    else dlg.querySelector('.dialog-body').appendChild(fmtRow);
  }

  function renderMediaFormats() {
    const seg = $('#adFmtSeg'), qw = $('#adQualityWrap'), note = $('#adFmtNote');
    if (!mediaSummary) return;
    const qualities = mediaSummary.qualities || [];
    seg.querySelectorAll('button').forEach((b) => {
      b.classList.toggle('on', b.dataset.f === mediaChoice.format);
      b.disabled = b.dataset.f === 'mp3' && !mediaSummary.has_audio_only && !mediaSummary.has_progressive;
    });
    if (mediaChoice.format === 'mp4' && qualities.length) {
      qw.innerHTML = `<select id="adQuality" aria-label="Video quality">
        <option value="best">Best available</option>
        ${qualities.map((q) => {
          const size = mediaSummary.sizesByHeight ? mediaSummary.sizesByHeight[q.height] : null;
          const sel = String(store.prefs.mediaQuality) === String(q.height) ? 'selected' : '';
          return `<option value="${q.height}${q.fps ? ':' + q.fps : ''}" ${sel}>${esc(q.label)}${size != null ? ' · ≈' + fmtBytes(size) : ''}</option>`;
        }).join('')}
      </select>`;
      const sel = qw.querySelector('#adQuality');
      sel.onchange = () => {
        if (sel.value === 'best') { mediaChoice.height = null; mediaChoice.fps = null; }
        else {
          const [h, fps] = sel.value.split(':').map(Number);
          mediaChoice.height = h; mediaChoice.fps = fps || null;
        }
      };
      // honour the Settings → Media default when the source offers it
      const prefHit = [...sel.options].find((o) =>
        o.value !== 'best' && o.value.split(':')[0] === String(store.prefs.mediaQuality));
      if (prefHit) { sel.value = prefHit.value; sel.onchange(); }
    } else {
      qw.innerHTML = '';
    }
    note.textContent = qualities.length
      ? 'Only qualities this source actually offers are listed.'
      : 'No video qualities reported — best available will be used.';
  }

  async function probeMediaFormats(url) {
    const box = $('#adMediaFmt');
    if (!box) return;
    box.style.display = 'block';
    $('#adFmtNote').textContent = 'Checking available formats…';
    try {
      mediaSummary = await engine.mediaFormats(url);
      mediaChoice.active = true;
      renderMediaFormats();
    } catch (e) {
      mediaChoice.active = false;
      $('#adFmtNote').textContent =
        'Format listing unavailable — best available will be used. (' + esc(friendly(e)) + ')';
    }
  }
  $('#adFmtSeg').onclick = (e) => {
    const b = e.target.closest('[data-f]');
    if (!b || b.disabled) return;
    mediaChoice.format = b.dataset.f;
    renderMediaFormats();
  };

  let inspecting = false;
  let lastInspected = '';
  async function doInspect() {
    const url = urlIn.value.trim();
    urlErr.style.display = 'none';
    if (!url || url === lastInspected) return;
    if (isTorrentSource(url)) {
      // torrent sources have no HTTP metadata to preview: name + size arrive
      // from the swarm once the metadata resolves
      lastInspected = url;
      prev.style.display = 'block';
      prev.className = 'preview ok';
      prev.innerHTML = `
        <div class="p-row"><span class="p-k">Type</span><span class="p-v">BitTorrent — magnet/torrent download</span></div>
        <div class="p-row"><span class="p-k">Note</span><span class="p-v">File name and size are resolved from the swarm after adding.</span></div>
      `;
      return;
    }
    if (!/^https?:\/\//i.test(url)) {
      prev.style.display = 'none';
      if (url) showUrlError('URL must start with http:// or https:// (magnet:?xt=… and .torrent links are also accepted)');
      return;
    }
    lastInspected = url;
    inspecting = true;
    prev.style.display = 'block';
    prev.className = 'preview';
    prev.innerHTML = `<span class="spin">${icon('loader', 14)} Inspecting resource…</span>`;
    try {
      const info = await engine.inspect(url, {});
      const size = info.content_length;
      prev.className = 'preview ok';
      prev.innerHTML = `
        <div class="p-row"><span class="p-k">Server status</span><span class="p-v">${info.status ?? '—'} — downloadable</span></div>
        <div class="p-row"><span class="p-k">File size</span><span class="p-v">${size != null ? fmtBytes(size) + (info.accept_ranges ? ' · resumable' : ' · single stream') : 'unknown'}</span></div>
        <div class="p-row"><span class="p-k">Type</span><span class="p-v">${esc(categoryOf(url.split('?')[0]))}</span></div>
        ${info.etag ? `<div class="p-row"><span class="p-k">ETag</span><span class="p-v" style="font-family:var(--mono);font-size:10.5px">${esc(String(info.etag).slice(0, 40))}</span></div>` : ''}
      `;
    } catch (e) {
      prev.className = 'preview bad';
      prev.innerHTML = `<div class="p-row"><span class="p-v" style="color:var(--err)">${esc(friendly(e))}</span></div>`;
    } finally {
      inspecting = false;
    }
    // media page (YouTube etc.): offer honest format + quality selection
    if (/youtube\.com|youtu\.be/i.test(url)) probeMediaFormats(url);
  }
  function showUrlError(msg) { urlErr.textContent = msg; urlErr.style.display = 'block'; }
  function friendly(e) {
    const map = {
      INVALID_URL: 'The engine cannot download from this URL scheme (http/https only).',
      UNSUPPORTED_URL: 'This looks like a media page. The optional yt-dlp extractor is required to resolve it (see Browser section).',
      TIMEOUT: 'The server took too long to answer.',
      CONNECT: 'Could not connect to the server.',
      DNS: 'Could not resolve the server address.',
    };
    const code = e instanceof EngineError ? e.code : 'ENGINE';
    return map[code] ? `${map[code]} (${code})` : e.message;
  }

  urlIn.addEventListener('input', () => { lastInspected = ''; });
  urlIn.addEventListener('change', doInspect);
  urlIn.addEventListener('paste', () => setTimeout(() => { doInspect(); }, 0));
  urlIn.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); start(); } });
  nameIn.addEventListener('input', () => {
    catIn.value = nameIn.value ? categoryOf(nameIn.value) : '';
  });

  function buildCfg() {
    const cfg = {};
    const name = nameIn.value.trim();
    if (name) cfg.filename = name;
    const dest = $('#adDest').value.trim();
    if (dest) { cfg.out_dir = dest; lastDest = dest; }
    const conns = $('#adConns').value;
    if (conns) cfg.max_connections = parseInt(conns, 10);
    const lim = $('#adLimit').value;
    if (lim) cfg.bandwidth_limit_bps = parseInt(lim, 10);
    const hash = $('#adHash').value.trim().toLowerCase();
    if (hash) {
      const algo = hash.length === 64 ? 'sha256' : hash.length === 96 ? 'blake3' : null;
      if (!algo) return { error: 'Checksum must be a 64-char sha256 or 96-char blake3 hex value.' };
      cfg.verify = { algo, expected: hash };
    }
    const ref = $('#adRef').value.trim();
    if (ref) cfg.referer = ref;
    const cookies = $('#adCookie').value.trim();
    if (cookies) {
      cfg.cookies = {};
      for (const pair of cookies.split(';')) {
        const i = pair.indexOf('=');
        if (i > 0) cfg.cookies[pair.slice(0, i).trim()] = pair.slice(i + 1).trim();
      }
    }
    if ($('#adOverwrite').checked) cfg.overwrite = true;
    if ($('#adAllowHtml').checked) cfg.allow_html = true;
    if ($('#adInsecure').checked) cfg.insecure_tls = true;
    return { cfg };
  }

  async function start() {
    const url = urlIn.value.trim();
    if (!url) { showUrlError('Enter a URL first.'); return; }
    if (!/^https?:\/\//i.test(url) && !isTorrentSource(url)) {
      showUrlError('URL must start with http:// or https:// (magnet:?xt=… and .torrent links are also accepted)');
      return;
    }
    if (isTorrentSource(url)) {
      $('#adMediaFmt').style.display = 'none';
      mediaChoice.active = false;
    }

    // duplicate check against the real list (engine also de-dupes destinations)
    const dupe = [...store.rows.values()].find((r) => r.url === url && !['failed', 'cancelled'].includes(r.state));
    if (dupe && !dupe.file_missing) {
      const again = await new Promise((resolve) => {
        const d2 = openDialog(`<div class="dialog" style="max-width:420px">
          <div class="dialog-head"><h3>Already in list</h3></div>
          <div class="dialog-body"><p style="color:var(--t2);font-size:13px">This URL is already downloading as <b>${esc(dupe.final_name)}</b> (${dupe.state}). Add it again anyway?</p></div>
          <div class="dialog-foot"><button class="btn" data-close>Cancel</button><button class="btn primary" data-ok>Add anyway</button></div>
        </div>`);
        d2.dlg.querySelector('[data-ok]').onclick = () => { d2.close(); resolve(true); };
        d2.overlay.addEventListener('mousedown', () => resolve(false));
      });
      if (!again) return;
    }

    const built = buildCfg();
    if (built.error) { showUrlError(built.error); return; }
    // chosen media format (YouTube etc.) — only when the probe succeeded
    if (mediaChoice.active) {
      built.cfg.media_format = mediaChoice.format;
      if (mediaChoice.format === 'mp4' && mediaChoice.height) {
        built.cfg.media_height = mediaChoice.height;
        if (mediaChoice.fps) built.cfg.media_fps = mediaChoice.fps;
      }
    }
    goBtn.disabled = true;
    goBtn.innerHTML = `<span class="btn-spinner"></span> Adding…`;
    try {
      const added = await engine.create(url, built.cfg);
      // "start paused" preference: add without starting (user starts in bulk)
      if (store.prefs.startDownloadsImmediately === false) {
        await refreshAll();
        toast('ok', 'Download added (paused)', added.final_name || url);
        close();
        return;
      }
      try { await engine.start(added.id); } catch (e) {
        if (!/ALREADY/i.test(String(e.message))) throw e;
      }
      await refreshAll();
      toast('ok', 'Download started', added.final_name || url);
      close();
    } catch (e) {
      goBtn.disabled = false;
      goBtn.innerHTML = `${icon('download', 14)} Download`;
      reportError(e, 'Could not add download');
    }
  }
  goBtn.onclick = start;
  if (prefillUrl) setTimeout(doInspect, 60);
}
