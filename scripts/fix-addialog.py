#!/usr/bin/env python3
"""addDialog.js: fix try/finally seam, clean media-format block, wire create cfg.
extract.rs: MP3 raw stream gets a honest raw name so conversion has no collision."""

P = '/home/z/my-project/download-engine/web/assets/screens/addDialog.js'
s = open(P).read()

# 1. fix seam: probe belongs AFTER finally
old_seam = """    }
    // media page (YouTube etc.): offer honest format + quality selection
    if (/youtube\\.com|youtu\\.be/i.test(url)) probeMediaFormats(url);
    } finally {
      inspecting = false;
    }
  }"""
new_seam = """    } finally {
      inspecting = false;
    }
    // media page (YouTube etc.): offer honest format + quality selection
    if (/youtube\\.com|youtu\\.be/i.test(url)) probeMediaFormats(url);
  }"""
assert old_seam in s, 'seam'
s = s.replace(old_seam, new_seam, 1)

# 2. replace whole inserted media block (from marker to end of probeMediaFormats)
start = s.index('  // ---- media format selection (YouTube): real qualities from the engine ----')
end = s.index('  function showUrlError(msg)')
new_block = """  // ---- media format selection (YouTube): real qualities from the engine ----
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

"""
s = s[:start] + new_block + s[end:]

# 3. wire mediaChoice into the create cfg + honor startDownloadsImmediately
old_start = """    const built = buildCfg();
    if (built.error) { showUrlError(built.error); return; }
    goBtn.disabled = true;
    goBtn.innerHTML = `<span class="btn-spinner"></span> Adding…`;
    try {
      const added = await engine.create(url, built.cfg);
      try { await engine.start(added.id); } catch (e) {
        if (!/ALREADY/i.test(String(e.message))) throw e;
      }
      await refreshAll();
      toast('ok', 'Download started', added.final_name || url);
      close();"""
new_start = """    const built = buildCfg();
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
      close();"""
assert old_start in s, 'start wiring'
s = s.replace(old_start, new_start, 1)

# 4. el import
s = s.replace(
  "import { openDialog, esc, reportError, actions } from '../ui.js';",
  "import { openDialog, el, esc, reportError, actions } from '../ui.js';", 1)

open(P, 'w').write(s)
print('addDialog.js: seam fixed, media block rewritten, create wired')

# ---- extract.rs: honest raw-stream name for the MP3 path ----
Q = '/home/z/my-project/download-engine/crates/engine-http/src/extract.rs'
r = open(Q).read()
old_name = '''            mime: Some("audio/mpeg".into()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            suggested_name: Some(format!("{}.mp3", title)),'''
new_name = '''            mime: Some("audio/mpeg".into()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            // the DOWNLOAD is the raw audio stream; conversion renames to .mp3
            suggested_name: Some(format!("{} (audio).{}", title, plan.video.ext)),'''
assert old_name in r, 'mp3 suggested_name'
r = r.replace(old_name, new_name, 1)
open(Q, 'w').write(r)
print('extract.rs: MP3 raw stream name fixed')
