#!/usr/bin/env python3
"""Velox UX/reliability batch patch — applies all fixes with exact-match assertions."""
import json, pathlib, sys

ROOT = pathlib.Path('/home/z/my-project/download-engine')
def patch(rel, old, new, must=True):
    p = ROOT / rel
    s = p.read_text(encoding='utf-8')
    if old not in s:
        if must:
            print(f'FAIL [{rel}]: pattern not found:\n---\n{old[:200]}\n---'); sys.exit(1)
        return
    p.write_text(s.replace(old, new, 1), encoding='utf-8')
    print(f'ok   [{rel}]')

# ============ 1. engine-media: explicit bundled-binary discovery ============
patch('crates/engine-media/src/pipeline.rs',
'''    pub async fn discover(port: Arc<P>) -> Result<Self, EngineError> {
        for exe in ["ffmpeg", "ffprobe"] {
            // Hold the sender for the whole run: dropping it closes the channel and
            // the port treats that as a cancel (kill-safe shutdown semantics).
            let (_kill_tx, kill_rx) = watch::channel(false);
            let version = port.version(exe, kill_rx).await.map_err(|_| {
                EngineError::new(
                    ErrorKind::MissingFfmpeg,
                    format!(
                        "{exe} not found — install an LGPL ffmpeg build \\
                                 (docs/FFMPEG_INTEGRATION.md §3)"
                    ),
                )
            })?;
            if exe == "ffmpeg" {
                verify_lgpl(&version)?;
            }
        }
        Ok(Self {
            port,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        })
    }''',
'''    pub async fn discover(port: Arc<P>) -> Result<Self, EngineError> {
        Self::discover_with(port, "ffmpeg".into(), "ffprobe".into()).await
    }

    /// Discovery with explicit binaries — the bundled-beside-exe layout wins
    /// over a bare PATH lookup, which missed the installer-shipped ffmpeg and
    /// left YouTube downloads as two separate files (merge port never injected).
    /// The LGPL gate still applies to whichever ffmpeg is selected.
    pub async fn discover_with(port: Arc<P>, ffmpeg: PathBuf, ffprobe: PathBuf) -> Result<Self, EngineError> {
        let ff_name = ffmpeg.display().to_string();
        let fp_name = ffprobe.display().to_string();
        for exe in [ff_name.as_str(), fp_name.as_str()] {
            // Hold the sender for the whole run: dropping it closes the channel and
            // the port treats that as a cancel (kill-safe shutdown semantics).
            let (_kill_tx, kill_rx) = watch::channel(false);
            let version = port.version(exe, kill_rx).await.map_err(|_| {
                EngineError::new(
                    ErrorKind::MissingFfmpeg,
                    format!(
                        "{exe} not found — install an LGPL ffmpeg build \\
                                 (docs/FFMPEG_INTEGRATION.md §3)"
                    ),
                )
            })?;
            if exe == ff_name {
                verify_lgpl(&version)?;
            }
        }
        Ok(Self {
            port,
            ffmpeg: ff_name,
            ffprobe: fp_name,
        })
    }''')

patch('crates/engine-media/src/lib.rs',
'''    pub async fn discover_subprocess() -> Result<Self, engine_core::error::EngineError> {
        Ok(Self {
            pipeline: MediaPipeline::discover(std::sync::Arc::new(SubprocessPort)).await?,
        })
    }''',
'''    pub async fn discover_subprocess() -> Result<Self, engine_core::error::EngineError> {
        Self::discover_subprocess_at(None).await
    }

    /// Prefer ffmpeg/ffprobe BUNDLED beside the app executable (the unified
    /// installer ships them next to Velox.exe), falling back to PATH lookup.
    /// This is the YouTube two-files fix: bare "ffmpeg" lookup missed the
    /// bundled binaries, the merge port was never injected, and both streams
    /// finished as separate files.
    pub async fn discover_subprocess_at(dir: Option<&std::path::Path>) -> Result<Self, engine_core::error::EngineError> {
        let name = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_string() };
        let pick = |n: &str| -> String {
            dir.and_then(|d| {
                let p = d.join(name(n));
                if p.exists() { Some(p.display().to_string()) } else { None }
            })
            .unwrap_or_else(|| n.to_string())
        };
        Ok(Self {
            pipeline: MediaPipeline::discover_with(
                std::sync::Arc::new(SubprocessPort),
                std::path::PathBuf::from(pick("ffmpeg")),
                std::path::PathBuf::from(pick("ffprobe")),
            )
            .await?,
        })
    }''')

patch('app/src-tauri/src/main.rs',
'            match tauri::async_runtime::block_on(engine_media::MergePort::discover_subprocess()) {',
'            match tauri::async_runtime::block_on(engine_media::MergePort::discover_subprocess_at(exe_dir.as_deref())) {')

# ============ 2. tauri.conf.json CSP: allow YouTube thumbnails ============
conf_p = ROOT / 'app/src-tauri/tauri.conf.json'
conf = json.loads(conf_p.read_text())
csp = conf['app']['security']['csp']
new_csp = csp.replace("img-src 'self' data:", "img-src 'self' data: https://i.ytimg.com")
assert new_csp != csp, 'CSP img-src pattern not found'
conf['app']['security']['csp'] = new_csp
conf_p.write_text(json.dumps(conf, indent=2, ensure_ascii=False) + '\n')
print('ok   [tauri.conf.json] CSP allows i.ytimg.com')

# ============ 3. api.js: shared user-readable message mapper ============
patch('web/assets/api.js',
'''/* ---------- desktop-only helpers ---------- */''',
'''/** Map a raw engine/yt-dlp message to a user-readable sentence (Windows
 * mess-error anatomy: problem + cause + next step; raw text stays in logs).
 * Shared by the store (failed events) and ui.js (action errors). */
export function friendlyEngineMessage(raw) {
  const s = String(raw || '');
  const low = s.toLowerCase();
  if (low.includes('ffmpeg') && (low.includes('not found') || low.includes('not installed') || low.includes('missing')))
    return 'Velox needs FFmpeg to combine video and audio into one file. Use the unified installer (it bundles FFmpeg), or add an LGPL FFmpeg build to your PATH.';
  if (low.includes('gpl'))
    return 'The installed FFmpeg is a GPL build, which Velox cannot use for licensing reasons. Install an LGPL build instead.';
  if (low.includes('merge interrupted'))
    return 'Combining the streams was interrupted — both files are kept. Press Retry to finish the merge.';
  if (low.includes('requested format is not available'))
    return 'This video no longer offers the selected quality. Try a different quality in the Add dialog.';
  if (low.includes('has already been downloaded'))
    return 'The file already exists — Velox kept the existing copy.';
  if (low.includes('timed out') || low.includes('timeout'))
    return 'The connection was too slow and timed out. Retrying continues from where it stopped.';
  if (low.includes('403') || low.includes('forbidden'))
    return 'The server refused the download — the link may have expired. Copy a fresh link from the source page.';
  if (low.includes('404') || low.includes('not found'))
    return 'The file is no longer available at this address.';
  if (low.includes('no space') || (low.includes('disk') && low.includes('full')))
    return 'There is not enough free disk space for this download.';
  if (low.includes('certificate'))
    return "The site's security certificate could not be verified.";
  if (low.includes('connection') && (low.includes('refused') || low.includes('reset')))
    return 'The connection was cut off. Retrying usually continues from where it stopped.';
  return s;
}

/* ---------- desktop-only helpers ---------- */''')

# ============ 4. store.js: notification quieting + friendly failed msg ============
patch('web/assets/store.js',
"import { engine, subscribeEvents, loadSettings, saveSettings, EVENT_NAMES } from './api.js';",
"import { engine, subscribeEvents, loadSettings, saveSettings, EVENT_NAMES, friendlyEngineMessage } from './api.js';")

patch('web/assets/store.js',
'''export function toast(kind, title, msg, actions = [], ttl = 6000) {
  const id = ++toastSeq;''',
'''export function toast(kind, title, msg, actions = [], ttl = 6000) {
  // anti-clutter: at most 4 toasts visible; the oldest non-error one is dropped
  // first (errors are the ones a user may still need to act on)
  while (store.toasts.length >= 4) {
    const idx = store.toasts.findIndex((t) => t.kind !== 'err');
    store.toasts.splice(idx >= 0 ? idx : 0, 1);
  }
  const id = ++toastSeq;''')

patch('web/assets/store.js',
'''        if (store.prefs.notifyCompleted) {
          toast('ok', 'Download completed', row?.final_name || `#${d.id}`, [
            { label: 'Open folder', act: () => window.__veloxActions?.reveal?.(d.id) },
          ]);
        }''',
'''        if (store.prefs.notifyCompleted) {
          // burst-batching: several completions within 1.5 s collapse into ONE
          // summary toast instead of a wall of near-identical popups
          completedBurst.names.push(row?.final_name || `#${d.id}`);
          completedBurst.ids.push(d.id);
          clearTimeout(completedBurst.timer);
          completedBurst.timer = setTimeout(() => {
            const names = completedBurst.names, ids = completedBurst.ids;
            completedBurst.names = []; completedBurst.ids = [];
            if (!names.length) return;
            if (names.length === 1) {
              toast('ok', 'Download completed', names[0], [
                { label: 'Open folder', act: () => window.__veloxActions?.reveal?.(ids[0]) },
              ]);
            } else {
              toast('ok', `${names.length} downloads completed`,
                names.slice(0, 5).join(', ') + (names.length > 5 ? ` +${names.length - 5} more` : ''), [], 7000);
            }
          }, 1500);
        }''')

patch('web/assets/store.js',
"/* ---------- toasts ---------- */\nlet toastSeq = 0;",
'''/* ---------- toasts ---------- */
let toastSeq = 0;
// completion burst-batcher (notification de-clutter): see the 'completed' case
const completedBurst = { names: [], ids: [], timer: null };''')

patch('web/assets/store.js',
"        const msg = d.message || 'download failed';",
"        const msg = friendlyEngineMessage(d.message || 'download failed');")

patch('web/assets/store.js',
'''        pushActivity(nowTs(), `Merge finished for <b>${esc(shortName(row?.final_name || `#${d.id}`))}</b>`);
        toast('ok', 'Merge complete', row?.final_name || `#${d.id}`);
        break;''',
'''        pushActivity(nowTs(), `Merge finished for <b>${esc(shortName(row?.final_name || `#${d.id}`))}</b>`);
        break; // no toast: the completion burst already announced the result''')

patch('web/assets/store.js',
'''        } else if (matches === true) {
          pushActivity(nowTs(), `Verification passed for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')})`);
          toast('ok', 'Verification passed', `${name} · ${d.algo || ''}`);
        } else {
          pushActivity(nowTs(), `Checksum computed for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')}) — no reference to compare`);
          toast('info', 'Checksum stored', `${name} · ${d.algo || ''} — no reference hash to compare against yet`);
        }''',
'''        } else if (matches === true) {
          pushActivity(nowTs(), `Verification passed for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')})`);
        } else {
          pushActivity(nowTs(), `Checksum computed for <b>${esc(shortName(name))}</b> (${esc(d.algo || '')}) — no reference to compare`);
        } // successes go to the activity feed only; only FAILURES interrupt''')

# ============ 5. updater.js: kill the racing exit; verify-on-next-launch ============
patch('web/assets/updater.js',
'''  // 3) download + verify + run installer (it waits for this app to exit)
  try {''',
'''  // 3) download + verify + run installer (it waits for this app to exit).
  // Record the attempt FIRST: on the next launch we compare versions and
  // honestly report "did not install" if the installer never completed
  // (the tauri-plugin-updater #3514 failure mode — app exits, nothing installs).
  try {
    localStorage.setItem('velox.update.attempt', JSON.stringify({
      from: (await currentAppVersion()) || '',
      to: pendingUpdate.version,
      at: Date.now(),
    }));
  } catch { /* private mode */ }
  try {''')

patch('web/assets/updater.js',
'''    // 4) the installer is waiting for this app to exit — comply gracefully.
    setUpdateState({ status: 'restarting' });
    pushActivity(nowTs(), 'Update installing — Velox restarts when the installer finishes…');
    const proc = tauriProcess();
    if (proc?.exit) setTimeout(() => { try { proc.exit(0); } catch { /* installer wait handles it */ } }, 600);
    else if (proc?.relaunch) setTimeout(() => { proc.relaunch().catch(() => {}); }, 600);
    return { ok: true };''',
'''    // 4) Windows: downloadAndInstall() already exited the process INSIDE the
    // plugin command (ShellExecuteW "/P /UPDATE /R" then std::process::exit) —
    // the JS promise never resolves there, and this code is unreachable on
    // Windows. The old 600ms exit(0) raced the installer spawn and produced
    // "app closes, update never installs". Reaching here means macOS/Linux:
    // relaunch directly, no competing timer.
    setUpdateState({ status: 'restarting' });
    pushActivity(nowTs(), 'Update installing — Velox restarts when the installer finishes…');
    const proc = tauriProcess();
    if (proc?.relaunch) { try { await proc.relaunch(); } catch { /* already exiting */ } }
    return { ok: true };''')

patch('web/assets/updater.js',
'''  setUpdateState({ status: 'idle' });
  // small startup reminder window; silent on failure (offline machines)''',
'''  setUpdateState({ status: 'idle' });
  // verify-on-next-launch: an attempt was recorded but this app is still the OLD
  // version -> the installer never completed. Say so once, honestly.
  try {
    const raw = localStorage.getItem('velox.update.attempt');
    if (raw) {
      const a = JSON.parse(raw);
      const cur = await currentAppVersion();
      if (cur && a?.to && cur !== a.to && isNewerVersion(a.to, cur)) {
        localStorage.removeItem('velox.update.attempt');
        setUpdateState({ status: 'error', error: `The update to ${a.to} did not finish installing` });
        toast('warn', 'Update not installed',
          `Velox is still on ${cur} — the update to ${a.to} did not complete. Try again from Settings → Updates.`, [], 10000);
      } else if (cur && a?.to && cur === a.to) {
        localStorage.removeItem('velox.update.attempt'); // install succeeded
      }
    }
  } catch { /* non-fatal */ }
  // small startup reminder window; silent on failure (offline machines)''')

# ============ 6. ui.js: thumbnails, dblclick, compact details, error map ============
patch('web/assets/ui.js',
"import { engine, EngineError, openFile, revealFolder, copyText, isDesktop } from './api.js';",
"import { engine, EngineError, openFile, revealFolder, copyText, isDesktop, friendlyEngineMessage } from './api.js';")

patch('web/assets/ui.js',
'''export function fileTypeIcon(name, size = 17) {
  const cat = categoryOf(name);
  const glyph = { video: 'fileVideo', audio: 'fileAudio', image: 'fileImage', archive: 'archive', document: 'fileText', software: 'package', code: 'fileCode', other: 'file' }[cat];
  return `<div class="file-ico ${cat === 'document' || cat === 'other' ? (cat === 'other' ? '' : 'doc') : cat}" title="${esc(cat)}">${icon(glyph, size)}</div>`;
}''',
'''export function fileTypeIcon(name, size = 17) {
  const cat = categoryOf(name);
  const glyph = { video: 'fileVideo', audio: 'fileAudio', image: 'fileImage', archive: 'archive', document: 'fileText', software: 'package', code: 'fileCode', other: 'file' }[cat];
  return `<div class="file-ico ${cat === 'document' || cat === 'other' ? (cat === 'other' ? '' : 'doc') : cat}" title="${esc(cat)}">${icon(glyph, size)}</div>`;
}

/** YouTube video id from a media-page URL, else null. */
export function ytThumbId(url) {
  const m = String(url || '').match(/(?:youtube\\.com\\/(?:watch\\?[^#]*v=|shorts\\/|embed\\/)|youtu\\.be\\/)([A-Za-z0-9_-]{11})/);
  return m ? m[1] : null;
}

/** Row art: a real YouTube thumbnail when the source is a video page, else the
 *  category icon. The img is attached by paintRow (CSP allows i.ytimg.com);
 *  on load error it falls back to the icon, so a dead thumb never leaves a hole. */
export function rowArt(r) {
  const id = ytThumbId(r?.url);
  if (!id) return fileTypeIcon(r?.final_name);
  return `<div class="file-ico thumb" data-thumb="${esc(id)}" title="${esc(r?.final_name || '')}"></div>`;
}''')

patch('web/assets/ui.js',
'''  node.innerHTML = `
    ${checkHtml}
    ${fileTypeIcon(r.final_name)}''',
'''  node.innerHTML = `
    ${checkHtml}
    ${rowArt(r)}''')

patch('web/assets/ui.js',
'''  node.querySelectorAll('[data-act]').forEach((b) => {''',
'''  // YouTube thumbnail: attach after innerHTML so a load error can fall back
  // to the category icon without inline handlers (CSP-safe)
  const thumbBox = node.querySelector('.file-ico.thumb');
  if (thumbBox) {
    const img = new Image();
    img.className = 'row-thumb';
    img.loading = 'lazy';
    img.alt = '';
    img.src = `https://i.ytimg.com/vi/${thumbBox.dataset.thumb}/mqdefault.jpg`;
    img.onerror = () => { thumbBox.classList.remove('thumb'); thumbBox.innerHTML = fileTypeIcon(r.final_name); };
    thumbBox.appendChild(img);
  }
  node.querySelectorAll('[data-act]').forEach((b) => {''')

patch('web/assets/ui.js',
'''  node.onclick = (e) => {
    if (e.target.closest('button, input')) return;
    window.__veloxOpenDetails?.(r.id);
  };
}''',
'''  // click = select (details are one deliberate action away), double-click =
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
}''')

patch('web/assets/ui.js',
"  items.push({ label: 'View details', icon: 'eye', k: 'Enter', act: () => window.__veloxOpenDetails?.(r.id) });",
"  items.push({ label: 'View details', icon: 'eye', act: () => window.__veloxOpenDetails?.(r.id) });")

patch('web/assets/ui.js',
'''function friendlyError(e) {
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
  };
  return map[code] ? `${map[code]} (${code}: ${e.message})` : e.message;
}''',
'''function friendlyError(e) {
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
}''')

# compact details drawer: primary facts + URL with copy + Advanced collapsed
ui_p = ROOT / 'web/assets/ui.js'
s = ui_p.read_text(encoding='utf-8')
start = s.index('export function paintDetails(node, r) {')
end = s.index('export { friendlyError };')
new_fn = '''export function paintDetails(node, r) {
  const title = node.querySelector('#dTitle');
  const body = node.querySelector('#dBody');
  title.textContent = r.final_name || `#${r.id}`;
  const kv = (k, v, dim = false) => `<div class="k"><span>${k}</span></div><div class="v ${dim ? 'dim' : ''}">${v}</div>`;
  const p = pct(r);
  const segs = (r.segments || []).map((s) => `<div class="seg-cell ${s.completed ? 'full' : s.pos > s.start ? 'done' : ''}" title="segment ${s.idx}: ${fmtBytes(s.pos)} / ${s.end != null ? fmtBytes(s.end - s.start + 1) : 'stream'}"></div>`).join('');
  const errs = (r.recent_errors || []).slice(0, 12).map((l) => `<div class="err-line">${esc(l)}</div>`).join('');
  const shortUrl = (u) => { const s = String(u || ''); return s.length > 46 ? s.slice(0, 46) + '…' : s; };
  body.innerHTML = `
    <div class="d-sec">
      <div class="kv">
        ${kv('Status', `<span class="chip ${({ created: 'queued' }[r.state] || r.state})">${({ created: 'queued', retrying: 'retrying' }[r.state] || r.state)}</span>`)}
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

'''
ui_p.write_text(s[:start] + new_fn + s[end:], encoding='utf-8')
print('ok   [ui.js] paintDetails -> compact + advanced disclosure')

# ============ 7. app.js: sidebar toggle + Ctrl+B + Enter opens file ============
patch('web/app.js',
'''  const foot = el(`<div class="side-foot"><span class="conn-dot"></span><span>connecting…</span></div>`);
  sideEl.appendChild(foot);
}''',
'''  const foot = el(`<div class="side-foot"><span class="conn-dot"></span><span>connecting…</span></div>`);
  sideEl.appendChild(foot);
  const tog = el(`<button class="icon-btn side-toggle" id="sideToggle" title="Toggle sidebar (Ctrl+B)" aria-label="Toggle sidebar">${icon('panelLeft', 14)}</button>`);
  tog.onclick = toggleSidebar;
  sideEl.appendChild(tog);
}

/** Collapse/expand the sidebar (icon-only rail). State persists via prefs. */
function toggleSidebar() {
  const collapsed = sideEl.classList.toggle('compact');
  setPrefs({ sidebarExpanded: !collapsed });
}''')

patch('web/app.js',
"  store, bootstrap, counts, subscribe, refreshAll, visibleRows, notify, toast, setPrefs,",
"  store, bootstrap, counts, subscribe, refreshAll, visibleRows, notify, toast, setPrefs,")

patch('web/app.js',
'''  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') { e.preventDefault(); $('#tbSearch')?.focus(); return; }''',
'''  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') { e.preventDefault(); $('#tbSearch')?.focus(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') { e.preventDefault(); toggleSidebar(); return; }''')

patch('web/app.js',
'''  if (e.key === 'Enter' && route === 'downloads' && store.selection.size === 1) {
    window.__veloxOpenDetails?.([...store.selection][0]);
  }''',
'''  if (e.key === 'Enter' && route === 'downloads' && store.selection.size === 1) {
    // Enter = the primary verb: open a finished file, otherwise inspect
    const id = [...store.selection][0];
    const r = store.rows.get(id);
    if (r && r.state === 'completed') actions.open(id);
    else window.__veloxOpenDetails?.(id);
  }''')

# ============ 8. app.css additions ============
css_p = ROOT / 'web/assets/app.css'
css_p.write_text(css_p.read_text(encoding='utf-8') + '''

/* ---- sidebar collapse (toggle button + Ctrl+B; state in prefs) ---- */
#sidebar { position: relative; transition: width .18s ease; overflow: visible; }
#sidebar.compact { width: 54px; }
#sidebar.compact .nav-label,
#sidebar.compact .nav-group,
#sidebar.compact .side-foot span:last-child { display: none; }
#sidebar.compact .nav-item { justify-content: center; }
.side-toggle { position: absolute; right: -13px; top: 16px; z-index: 6;
  background: var(--bg2, #16222e); border: 1px solid var(--line, #24384c);
  width: 26px; height: 26px; display: flex; align-items: center; justify-content: center; }

/* ---- row thumbnails (YouTube) ---- */
.file-ico.thumb { position: relative; overflow: hidden; }
.row-thumb { position: absolute; inset: 0; width: 100%; height: 100%;
  object-fit: cover; border-radius: 6px; }

/* ---- details drawer: compact URL line + advanced disclosure ---- */
.url-line { display: flex; align-items: center; gap: 6px; min-width: 0; }
.url-line .u { overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  max-width: 210px; direction: ltr; font-family: var(--mono, monospace); font-size: 11px; }
.icon-btn.sm { width: 22px; height: 22px; display: inline-flex; align-items: center;
  justify-content: center; flex: none; }
details.adv > summary { cursor: pointer; color: var(--t2); font-size: 12px;
  padding: 8px 0 4px; user-select: none; list-style: revert; }
details.adv > summary:hover { color: var(--t1); }
''')
print('ok   [app.css] additions appended')

print('ALL PATCHES APPLIED')
