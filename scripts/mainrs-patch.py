#!/usr/bin/env python3
"""main.rs: bin/ tool layout, theme commands, Linux browser detection."""

P = '/home/z/my-project/download-engine/app/src-tauri/src/main.rs'
s = open(P).read()

# ---------- 1. AppState: themes_dir ----------
old = """struct AppState {
    manager: Arc<DownloadManager>,
    downloads_root: PathBuf,
    /// Persisted UI preferences (JSON; written by settings_set).
    settings_path: PathBuf,
    /// Set by the first CloseRequested; makes repeated close requests safe.
    shutting_down: AtomicBool,
}"""
new = """struct AppState {
    manager: Arc<DownloadManager>,
    downloads_root: PathBuf,
    /// Persisted UI preferences (JSON; written by settings_set).
    settings_path: PathBuf,
    /// User-installed third-party themes (JSON manifests; docs/THEMES.md).
    themes_dir: PathBuf,
    /// Set by the first CloseRequested; makes repeated close requests safe.
    shutting_down: AtomicBool,
}

/// Locate a bundled tool or sidecar: `bin/` beside the executable FIRST (the
/// 0.7.0+ install layout keeps dependencies separated and identifiable), then
/// beside the exe (0.6.x flat layout — updates over old installs keep working),
/// then PATH.
fn bundled_tool_path(name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    if let Some(d) = exe_dir() {
        let b = d.join("bin").join(&exe_name);
        if b.exists() {
            return Some(b);
        }
        let flat = d.join(&exe_name);
        if flat.exists() {
            return Some(flat);
        }
    }
    find_on_path(name)
}"""
assert old in s, 'AppState'
s = s.replace(old, new, 1)

# ---------- 2. theme commands after settings_set ----------
old2 = """/* ================= browser-extension installer (in-app setup) ================="""
new2 = """/* ================= third-party themes (Settings → Appearance) =================
 * Themes are DATA, never code: JSON manifests of CSS custom properties
 * (docs/THEMES.md). The full security/contrast validation lives in the
 * frontend sanitizer (assets/themes.js); these commands only gate what the
 * filesystem touches: a sane id, a colors object, a size cap.
 * ==================================================================== */
const THEMES_MAX_BYTES: usize = 100 * 1024;

#[tauri::command]
fn themes_dir(state: State<'_, AppState>) -> Result<String, String> {
    std::fs::create_dir_all(&state.themes_dir).map_err(|e| e.to_string())?;
    Ok(state.themes_dir.display().to_string())
}

/// List installed third-party themes (raw manifests; the UI validates each).
#[tauri::command]
fn themes_list(state: State<'_, AppState>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(&state.themes_dir) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(txt) = std::fs::read_to_string(&p) else { continue };
        if txt.len() > THEMES_MAX_BYTES {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { continue };
        let id = v.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        out.push(serde_json::json!({ "id": id, "name": name, "theme": v }));
    }
    out
}

fn valid_theme_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 32 {
        return Err("theme id must be 1-32 characters".into());
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("theme id allows letters, digits, '_' and '-' only".into());
    }
    if matches!(id, "dark" | "light" | "purple" | "blue") {
        return Err("this id collides with a built-in theme".into());
    }
    Ok(())
}

/// Install (persist) a third-party theme manifest from its JSON text.
#[tauri::command]
fn theme_install(state: State<'_, AppState>, json: String) -> Result<serde_json::Value, String> {
    if json.len() > THEMES_MAX_BYTES {
        return Err("theme file too large (100 KB limit)".into());
    }
    let v: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("not valid JSON: {e}"))?;
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or("theme manifest needs an id")?;
    valid_theme_id(id)?;
    let colors = v.get("colors").ok_or("theme manifest needs a colors object")?;
    if !colors.is_object() {
        return Err("colors must be an object of variable → color".into());
    }
    for (k, val) in colors.as_object().unwrap() {
        if !k.starts_with("--") {
            return Err(format!("unexpected key '{k}' in colors (variables start with --)"));
        }
        if !val.is_string() {
            return Err(format!("value for {k} must be a string color"));
        }
    }
    std::fs::create_dir_all(&state.themes_dir).map_err(|e| e.to_string())?;
    let path = state.themes_dir.join(format!("{id}.json"));
    std::fs::write(&path, &json).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "id": id, "path": path.display().to_string() }))
}

/// Remove an installed third-party theme.
#[tauri::command]
fn theme_uninstall(state: State<'_, AppState>, id: String) -> Result<(), String> {
    valid_theme_id(&id)?;
    let p = state.themes_dir.join(format!("{id}.json"));
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/* ================= browser-extension installer (in-app setup) ================="""
assert old2 in s, 'theme cmd anchor'
s = s.replace(old2, new2, 1)

# ---------- 3. extension_setup + extension_status: sidecar via bin/ ----------
old3 = """    let host = exe_dir()
        .map(|d| {
            d.join(if cfg!(windows) { "velox-browser-host.exe" } else { "velox-browser-host" })
        })
        .filter(|p| p.exists());"""
new3 = """    let host = bundled_tool_path("velox-browser-host");"""
assert old3 in s, 'setup host path'
s = s.replace(old3, new3, 1)

old4 = """        "host_exe_present": exe_dir()
            .map(|d| d.join(if cfg!(windows) { "velox-browser-host.exe" } else { "velox-browser-host" }).exists())
            .unwrap_or(false),"""
new4 = """        "host_exe_present": bundled_tool_path("velox-browser-host").is_some(),"""
assert old4 in s, 'status host path'
s = s.replace(old4, new4, 1)

# ---------- 4. setup: tools in bin/ first, PATH prepend the right dir ----------
old5 = """            let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
            if let Some(d) = &exe_dir {
                let bundled = d.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }).exists()
                    || d.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }).exists();
                if bundled {
                    let sep = if cfg!(windows) { ';' } else { ':' };
                    let old_path = std::env::var("PATH").unwrap_or_default();
                    std::env::set_var("PATH", format!("{}{sep}{}", d.display(), old_path));
                }
            }
            let extractor = exe_dir
                .as_ref()
                .map(|d| d.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }))
                .filter(|p| p.exists())
                .unwrap_or_else(|| PathBuf::from("yt-dlp"));"""
new5 = """            // 0.7.0+ layout: bundled dependencies live in bin/ beside the exe
            // (flat 0.6.x layout stays discoverable for in-place updates).
            let tools_dir = exe_dir()
                .map(|d| {
                    let bin = d.join("bin");
                    let has = |p: &PathBuf| {
                        p.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }).exists()
                            || p.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }).exists()
                    };
                    if has(&bin) { bin } else { d.to_path_buf() }
                });
            if let Some(d) = &tools_dir {
                let sep = if cfg!(windows) { ';' } else { ':' };
                let old_path = std::env::var("PATH").unwrap_or_default();
                std::env::set_var("PATH", format!("{}{sep}{}", d.display(), old_path));
            }
            let extractor = tools_dir
                .as_ref()
                .map(|d| d.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }))
                .filter(|p| p.exists())
                .unwrap_or_else(|| PathBuf::from("yt-dlp"));"""
assert old5 in s, 'setup tools'
s = s.replace(old5, new5, 1)

# merge discovery: point at the dir that actually holds ffmpeg
old6 = """match tauri::async_runtime::block_on(engine_media::MergePort::discover_subprocess_at(exe_dir.as_deref())) {"""
new6 = """match tauri::async_runtime::block_on(engine_media::MergePort::discover_subprocess_at(tools_dir.as_deref())) {"""
assert old6 in s, 'merge discover'
s = s.replace(old6, new6, 1)

# ---------- 5. AppState init: themes_dir ----------
old7 = """            app.manage(AppState {
                manager: manager.clone(),"""
new7 = """            let themes_dir = app.path().app_data_dir()?.join("themes");
            std::fs::create_dir_all(&themes_dir).map_err(|e| e.to_string())?;
            app.manage(AppState {
                manager: manager.clone(),"""
assert old7 in s, 'state init'
s = s.replace(old7, new7, 1)
# add the field init right after settings_path init in the literal
import re
m = re.search(r'app\.manage\(AppState \{\n(.*?)\n            \}\);', s, re.S)
assert m, 'state literal'
lit = m.group(0)
if 'themes_dir,' not in lit:
    lit2 = lit.replace('settings_path,', 'settings_path,\n                themes_dir,')
    assert lit2 != lit
    s = s.replace(lit, lit2, 1)

# ---------- 6. invoke_handler: register theme commands ----------
old8 = """        browser_detect,
        extension_setup,
        extension_status
    ])"""
new8 = """        browser_detect,
        extension_setup,
        extension_status,
        themes_dir,
        themes_list,
        theme_install,
        theme_uninstall
    ])"""
assert old8 in s, 'handler'
s = s.replace(old8, new8, 1)

# ---------- 7. detect_browsers: Linux filesystem branch ----------
anchor = """/// Filesystem + (Windows) registry discovery of installed browsers.
fn detect_browsers() -> Vec<BrowserInfo> {
    #[allow(unused_mut)]
    let mut out = Vec::new();"""
assert anchor in s, 'detect anchor'
linux_block = anchor + """
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Filesystem discovery: well-known binary names on PATH + per-user
        // config directories (Chromium NM manifests live under ~/.config/<dir>).
        let home = std::env::var("HOME").unwrap_or_default();
        let which = |n: &str| -> bool {
            let Ok(path) = std::env::var("PATH") else { return false };
            path.split(':')
                .map(|d| std::path::Path::new(d).join(n))
                .any(|p| p.exists())
        };
        struct Probe {
            id: &'static str,
            name: &'static str,
            bins: &'static [&'static str],
            cfg_dir: &'static str,
            page: &'static str,
        }
        let probes = [
            Probe { id: "chrome", name: "Google Chrome", bins: &["google-chrome-stable", "google-chrome"], cfg_dir: "google-chrome", page: "chrome://extensions" },
            Probe { id: "edge", name: "Microsoft Edge", bins: &["microsoft-edge-stable", "microsoft-edge"], cfg_dir: "microsoft-edge", page: "edge://extensions" },
            Probe { id: "brave", name: "Brave", bins: &["brave-browser", "brave"], cfg_dir: "BraveSoftware/Brave-Browser", page: "brave://extensions" },
            Probe { id: "vivaldi", name: "Vivaldi", bins: &["vivaldi", "vivaldi-stable"], cfg_dir: "vivaldi", page: "vivaldi://extensions" },
            Probe { id: "chromium", name: "Chromium", bins: &["chromium", "chromium-browser"], cfg_dir: "chromium", page: "chrome://extensions" },
            Probe { id: "opera", name: "Opera", bins: &["opera"], cfg_dir: "opera", page: "opera://extensions" },
        ];
        for p in &probes {
            let bin = p.bins.iter().find(|b| which(b));
            let cfg_present =
                std::path::Path::new(&home).join(".config").join(p.cfg_dir).exists();
            if bin.is_some() || cfg_present {
                out.push(BrowserInfo {
                    id: p.id.to_string(),
                    name: p.name.to_string(),
                    family: "chromium".into(),
                    exe: bin.map(|b| b.to_string()).unwrap_or_default(),
                    extensions_page: p.page.to_string(),
                });
            }
        }
        if which("firefox") {
            out.push(BrowserInfo {
                id: "firefox".into(),
                name: "Firefox".into(),
                family: "firefox".into(),
                exe: "firefox".into(),
                extensions_page: "about:debugging#/runtime/this-firefox".into(),
            });
        }
    }"""
s = s.replace(anchor, linux_block, 1)

open(P, 'w').write(s)
print('main.rs patched: bin/ discovery, theme commands, Linux browser detect')
