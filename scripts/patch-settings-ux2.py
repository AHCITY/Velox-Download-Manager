#!/usr/bin/env python3
"""Patch settings.js for UX 2.0: real default-dir picker, create-time defaults."""
import pathlib

p = pathlib.Path('/home/z/my-project/download-engine/web/assets/screens/settings.js')
s = p.read_text()

# api import: add pickDirectory helper (implemented in api.js next)
old = "import { isDesktop } from '../api.js';"
new = "import { isDesktop, defaultDownloadDir, pickDirectory } from '../api.js';"
assert old in s, 'api import'
s = s.replace(old, new)

# --- General: real default-directory row (native picker on desktop)
old = """      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default download directory</div><div class="s-desc">Fixed by the engine at startup (desktop app policy: the OS Downloads folder).</div></div>
        <div class="s-ctl"><span class="tag" style="max-width:280px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${esc(store.engine?.default_out_dir || '')}">${esc(store.engine?.default_out_dir || '—')}</span></div></div>`),
    );
  } else if (section === 'downloads') {"""
new = """      defaultDirRow(),
    );
  } else if (section === 'downloads') {"""
assert old in s, 'general dir row'
s = s.replace(old, new)

# --- Downloads: replace the Home-only pref with real create-time defaults
old = """      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Row density</div><div class="s-desc">Compact rows show more downloads per screen.</div></div>
        <div class="s-ctl"><select id="seDensity"><option value="comfortable" ${P.density === 'comfortable' ? 'selected' : ''}>Comfortable</option><option value="compact" ${P.density === 'compact' ? 'selected' : ''}>Compact</option></select></div></div>`),
      toggleRow('autoClearCompleted', 'Hide completed from Home after a while', 'Completed entries slide out of the Home active list shortly after finishing (they stay in Downloads/History).', P.autoClearCompleted, (v) => setPrefs({ autoClearCompleted: v })),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Maximum concurrent downloads</div><div class="s-desc">Engine configuration — how many downloads run in parallel.</div></div>
        <div class="s-ctl"><span class="tag acc">${store.engine?.max_concurrent_downloads ?? '—'}</span></div></div>`),"""
new = """      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Row density</div><div class="s-desc">Compact rows show more downloads per screen. Applies to the Downloads list.</div></div>
        <div class="s-ctl"><select id="seDensity"><option value="comfortable" ${P.density === 'comfortable' ? 'selected' : ''}>Comfortable</option><option value="compact" ${P.density === 'compact' ? 'selected' : ''}>Compact</option></select></div></div>`),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default connections per download</div><div class="s-desc">Pre-selected in the Add Download dialog. The engine clamps this to its per-host ceiling.</div></div>
        <div class="s-ctl"><select id="seDefConns">
          <option value="" ${!P.defaultConnections ? 'selected' : ''}>Engine default</option>
          ${[1, 2, 4, 8, 16].map((n) => `<option value="${n}" ${String(P.defaultConnections) === String(n) ? 'selected' : ''}>${n}</option>`).join('')}
        </select></div></div>`),
      toggleRow('defaultOverwrite', 'Overwrite existing files by default', 'Pre-checks "Overwrite existing file" in the Add dialog. Otherwise numbered names (file(1).ext) are chosen.', P.defaultOverwrite, (v) => setPrefs({ defaultOverwrite: v })),
      el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Maximum concurrent downloads</div><div class="s-desc">Engine configuration — how many downloads run in parallel (fixed at startup).</div></div>
        <div class="s-ctl"><span class="tag acc">${store.engine?.max_concurrent_downloads ?? '—'}</span></div></div>`),"""
assert old in s, 'downloads rows'
s = s.replace(old, new)

old = """    const dens = card.querySelector('#seDensity');
    if (dens) dens.onchange = (e) => setPrefs({ density: e.target.value });
  } else if (section === 'network') {"""
new = """    const dens = card.querySelector('#seDensity');
    if (dens) dens.onchange = (e) => setPrefs({ density: e.target.value });
    const conns = card.querySelector('#seDefConns');
    if (conns) conns.onchange = (e) => setPrefs({ defaultConnections: e.target.value });
  } else if (section === 'network') {"""
assert old in s, 'downloads wiring'
s = s.replace(old, new)

# --- appearance: shortcuts list gains Ctrl+V
old = '<span class="kbd">Ctrl N</span> add download &nbsp; <span class="kbd">Ctrl F</span> search'
new = '<span class="kbd">Ctrl N</span> add download &nbsp; <span class="kbd">Ctrl V</span> paste URL &nbsp; <span class="kbd">Ctrl F</span> search'
assert old in s, 'kbd'
s = s.replace(old, new)

# --- append the defaultDirRow builder before export
old = "export { renderSettings as render };"
new = """// Default download directory: a REAL native folder picker on desktop; the
// choice becomes the pre-filled destination for NEW downloads (Add dialog)
// and persists across restarts. The engine's startup dir remains the fallback.
function defaultDirRow() {
  const eff = store.prefs.defaultDir || store.engine?.default_out_dir || '';
  const row = el(`<div class="setting-row"><div class="s-txt"><div class="s-name">Default download directory</div>
      <div class="s-desc">Pre-filled destination for new downloads. New downloads land here unless you change the folder in the Add dialog.</div>
      <div class="s-desc" style="font-family:var(--mono);font-size:11px;margin-top:4px;word-break:break-all" id="seDirVal"></div></div>
      <div class="s-ctl"><button class="btn sm" id="sePickDir">${icon('folderOpen', 13)} Choose…</button></div></div>`);
  const val = row.querySelector('#seDirVal');
  const paint = (d) => { val.textContent = d || '—'; val.title = d || ''; };
  paint(eff);
  row.querySelector('#sePickDir').onclick = async () => {
    const chosen = await pickDirectory(store.prefs.defaultDir || store.engine?.default_out_dir || '');
    if (!chosen) return; // cancelled
    setPrefs({ defaultDir: chosen });
    paint(chosen);
    toast('ok', 'Default directory updated', chosen);
  };
  if (isDesktop && !store.prefs.defaultDir) {
    // desktop first run: show the OS dir the engine would use
    defaultDownloadDir().then((d) => { if (d && !store.prefs.defaultDir) paint(d); });
  }
  return row;
}
export { renderSettings as render };"""
assert old in s, 'export'
s = s.replace(old, new)

p.write_text(s)
print('settings.js patched')
