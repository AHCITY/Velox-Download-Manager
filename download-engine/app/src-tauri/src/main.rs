// Velox desktop application — Tauri 2 shell around the EXISTING Rust engine.
// The engine runs IN-PROCESS: no localhost HTTP service, no sidecar process,
// no second downloader. The browser preview (`velox serve-http`) remains a
// separate, opt-in transport for CLI/testing/remote use.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use engine_bridge::wire_event;
use engine_core::config::EngineConfig;
use engine_core::manager::DownloadManager;
use engine_http::ReqwestBackend;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;

/// Monotonic ms timestamp of the last AUTHENTICATED bridge request. The
/// frontend uses it to show "extension connected" honestly (a live host makes
/// whitelisted calls; silence means not connected). 0 = never seen.
static BRIDGE_LAST_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Managed engine handle shared by all commands.
struct AppState {
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
}

/// Route a JSON-RPC 2.0 request through the EXISTING engine API (engine-api).
/// The frontend builds the exact same envelopes it sends to /rpc in browser
/// preview mode, so one request/response contract serves both transports.
#[tauri::command]
async fn rpc(state: State<'_, AppState>, body: String) -> Result<String, String> {
    // light sanity bound: reject absurd request bodies early (mirrors the HTTP cap)
    if body.len() > 1_000_000 {
        return Err("request too large".into());
    }
    engine_api::handle_line(&state.manager, &body)
        .await
        .ok_or_else(|| "request produced no response".to_string())
}

/// The engine's default download directory (shown in Settings and used as the
/// fallback destination). Resolved from the OS at startup, never inside the
/// installation directory.
#[tauri::command]
fn default_download_dir(state: State<'_, AppState>) -> String {
    state.downloads_root.display().to_string()
}

/// Desktop integration: open a completed download or its folder with the OS
/// default handler. Input is validated in Rust on the FINAL path that will be
/// opened: `reveal` resolves the parent directory first, then BOTH sides are
/// canonicalized so symlink/junction traversal cannot escape the downloads
/// root — a compromised frontend cannot open arbitrary filesystem locations.
#[tauri::command]
fn open_path(state: State<'_, AppState>, path: String, reveal: bool) -> Result<(), String> {
    let requested = PathBuf::from(&path);
    // resolve the FINAL target before validating: reveal opens the containing
    // directory (for files); directories open themselves
    let final_path = if reveal {
        match requested.parent() {
            Some(p) => p.to_path_buf(),
            None => requested.clone(),
        }
    } else {
        requested.clone()
    };
    // canonicalize resolves symlinks/junctions and doubles as an existence check
    let resolved = std::fs::canonicalize(&final_path)
        .map_err(|_| "file or folder no longer exists".to_string())?;
    let root = std::fs::canonicalize(&state.downloads_root)
        .map_err(|e| format!("download directory is unavailable: {e}"))?;
    let in_root = resolved.starts_with(&root);
    if !in_root {
        // trust anchor is the engine database, not the webview: a path is openable
        // when it is the recorded artifact (dest_dir/final_name) or its containing
        // directory of a download the ENGINE created — custom destinations included
        let recorded = state
            .manager
            .db()
            .list_downloads()
            .unwrap_or_default()
            .iter()
            .any(|row| {
                let dest = std::path::Path::new(&row.dest_dir);
                let file = dest.join(&row.final_name);
                std::fs::canonicalize(&file).map(|c| c == resolved).unwrap_or(false)
                    || std::fs::canonicalize(dest).map(|c| c == resolved).unwrap_or(false)
            });
        if !recorded {
            return Err("path is outside the configured download directories".into());
        }
    }
    // Launch with the SHELL form of the validated path. `resolved` is canonical,
    // which on Windows means verbatim (\\?\C:\…) — a form the Win32 file APIs
    // accept but explorer.exe does not, so "Open file" / "Open folder" opened
    // nothing. Validation above stays on the canonical form; only the argument
    // handed to the shell is simplified.
    let launch = engine_core::paths::shell_path(&resolved);
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&launch)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&launch)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&launch)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// UI preferences: read the persisted JSON (empty string when absent — first run).
#[tauri::command]
fn settings_get(state: State<'_, AppState>) -> Result<String, String> {
    std::fs::read_to_string(&state.settings_path).or(Ok(String::new()))
}

/// UI preferences: persist the JSON payload (bounded — settings are small).
#[tauri::command]
fn settings_set(state: State<'_, AppState>, json: String) -> Result<(), String> {
    if json.len() > 256 * 1024 {
        return Err("settings payload too large".into());
    }
    std::fs::write(&state.settings_path, json).map_err(|e| e.to_string())
}

/* ================= third-party themes (Settings → Appearance) =================
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

/* ================= browser-extension installer (in-app setup) =================
 * The desktop app drives the FULL browser integration setup: it stages the
 * bundled extension into a stable per-user location, registers the native
 * messaging host for the selected browser(s), and opens that browser's
 * extensions page. Chromium does not permit fully silent unpacked installs
 * (its security model; every mainstream downloader uses the same guided
 * flow), so "install" = everything automatable + the exact two clicks the
 * user must do, with live verification once the extension connects.
 * ==================================================================== */
// Derived from the shipped manifest key (engine_core::extension) — the ID Chrome
// actually computes. A stale literal here makes the NM host manifest allowlist an
// origin Chrome never grants: every connectNative fails with "forbidden".
use engine_core::extension::EXTENSION_ID;

#[derive(serde::Serialize)]
struct BrowserInfo {
    id: String,
    name: String,
    family: String, // chromium | firefox
    exe: String,    // empty when not found
    extensions_page: String,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

/// The packaged extension ships BESIDE the app exe (NSIS layout). Dev builds
/// fall back to the workspace folder.
fn extension_source_dir() -> Option<PathBuf> {
    if let Some(d) = exe_dir() {
        let c = d.join("extension");
        if c.join("manifest.json").exists() {
            return Some(c);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extension");
    dev.join("manifest.json").exists().then_some(dev)
}

/// Stable per-user install location for the extension (survives updates).
fn extension_install_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("Velox").join("extension")
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|_| PathBuf::from("."));
        base.join("io.velox.downloadmanager").join("extension")
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Where msedge.exe lives: the system install (Program Files (x86)) AND the
/// per-user install under %LOCALAPPDATA% — the second form is common on
/// machines without admin rights and was previously missed, so Edge users got
/// "Microsoft Edge is not installed (or could not be located)" with Edge
/// present (RC-4).
#[cfg_attr(not(windows), allow(dead_code))]
fn edge_exe_candidates(pf86: &str, lad: &str) -> Vec<String> {
    vec![
        format!(r"{}\Microsoft\Edge\Application\msedge.exe", pf86),
        format!(r"{}\Microsoft\Edge\Application\msedge.exe", lad),
    ]
}

/// Filesystem + (Windows) registry discovery of installed browsers.
fn detect_browsers() -> Vec<BrowserInfo> {
    #[allow(unused_mut)]
    let mut out = Vec::new();
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
    }
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
        let reg_exe = |sub: &str| -> Option<PathBuf> {
            let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
            hk.open_subkey_with_flags(sub, KEY_READ)
                .ok()?
                .get_value::<String, _>("")
                .ok()
                .map(PathBuf::from)
        };
        let fs_exe = |candidates: &[&str]| -> Option<PathBuf> {
            candidates.iter().map(PathBuf::from).find(|p| p.exists())
        };
        let resolve = |app_paths: &str, candidates: &[&str]| -> String {
            reg_exe(app_paths)
                .or_else(|| fs_exe(candidates))
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        };
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();
        let lad = std::env::var("LOCALAPPDATA").unwrap_or_default();
        out.push(BrowserInfo {
            id: "chrome".into(),
            name: "Google Chrome".into(),
            family: "chromium".into(),
            exe: resolve(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe",
                &[
                    &format!("{}\\Google\\Chrome\\Application\\chrome.exe", pf86),
                    &format!("{}\\Google\\Chrome\\Application\\chrome.exe", pf),
                    &format!("{}\\Google\\Chrome\\Application\\chrome.exe", lad),
                ],
            ),
            extensions_page: "chrome://extensions".into(),
        });
        out.push(BrowserInfo {
            id: "edge".into(),
            name: "Microsoft Edge".into(),
            family: "chromium".into(),
            exe: resolve(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe",
                &edge_exe_candidates(&pf86, &lad)
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
            ),
            extensions_page: "edge://extensions".into(),
        });
        out.push(BrowserInfo {
            id: "brave".into(),
            name: "Brave".into(),
            family: "chromium".into(),
            exe: resolve(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\brave.exe",
                &[
                    &format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", pf86),
                    &format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", lad),
                ],
            ),
            extensions_page: "brave://extensions".into(),
        });
        out.push(BrowserInfo {
            id: "opera".into(),
            name: "Opera".into(),
            family: "chromium".into(),
            exe: fs_exe(&[
                &format!("{}\\Programs\\Opera\\opera.exe", lad),
                &format!("{}\\Programs\\Opera GX\\opera.exe", lad),
            ])
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
            extensions_page: "opera://extensions".into(),
        });
        out.push(BrowserInfo {
            id: "vivaldi".into(),
            name: "Vivaldi".into(),
            family: "chromium".into(),
            exe: fs_exe(&[&format!("{}\\Vivaldi\\Application\\vivaldi.exe", lad)])
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            extensions_page: "vivaldi://extensions".into(),
        });
        out.push(BrowserInfo {
            id: "firefox".into(),
            name: "Firefox".into(),
            family: "firefox".into(),
            exe: resolve(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\firefox.exe",
                &[&format!("{}\\Mozilla Firefox\\firefox.exe", pf)],
            ),
            extensions_page: "about:debugging#/runtime/this-firefox".into(),
        });
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for (id, name, bin, page) in [
            ("chrome", "Google Chrome", "google-chrome", "chrome://extensions"),
            ("edge", "Microsoft Edge", "microsoft-edge", "chrome://extensions"),
            ("brave", "Brave", "brave-browser", "brave://extensions"),
            ("opera", "Opera", "opera", "opera://extensions"),
            ("vivaldi", "Vivaldi", "vivaldi", "vivaldi://extensions"),
            ("chromium", "Chromium", "chromium", "chrome://extensions"),
            ("firefox", "Firefox", "firefox", "about:debugging"),
        ] {
            let path = PathBuf::from("/usr/bin").join(bin);
            if path.exists() {
                out.push(BrowserInfo {
                    id: id.into(),
                    name: name.into(),
                    family: if id == "firefox" { "firefox".into() } else { "chromium".into() },
                    exe: path.display().to_string(),
                    extensions_page: page.into(),
                });
            }
        }
    }
    out
}

#[tauri::command]
fn browser_detect() -> Vec<BrowserInfo> {
    detect_browsers()
}

/* ---------------- native-host registration probe ----------------
 * A registry KEY existing is not "registered": an older install (or an
 * uninstalled machine migration) can leave a default value pointing at a
 * missing manifest or a dead exe, and the UI then shows a green light while
 * every connectNative fails. The probe reads the actual registration, loads
 * the manifest file, and validates its CONTENT against what Chrome/Edge
 * enforce — the concrete issues drive the Browser screen's repair flow.
 * ==================================================================== */

/// HKCU\Software subkeys whose NativeMessagingHosts\io.velox.downloadmanager
/// key receives the manifest path. Mirrors velox-host's WINDOWS_BROWSER_KEYS
/// (kept in sync by velox-host's own tests).
#[cfg_attr(not(windows), allow(dead_code))]
const HOST_REG_BROWSERS: &[&str] = &[
    "Google\\Chrome",
    "Microsoft\\Edge",
    "BraveSoftware\\Brave-Browser",
    "Opera Software",
    "Chromium",
    "Vivaldi",
];

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(serde::Serialize)]
struct HostRegistration {
    /// at least one browser's NM key points at a manifest path
    registered: bool,
    /// the manifest path the (first found) registration points at
    manifest_path: String,
    /// the manifest file exists AND its content satisfies Chrome/Edge rules
    manifest_valid: bool,
    /// concrete problems (empty when valid) — what "Repair" fixes
    issues: Vec<String>,
}

/// Validate a native-host manifest's content. Returns the host executable
/// path, or the concrete issues (name/type/path/allowed_origins per the
/// Chrome and Edge native-messaging specs).
#[cfg_attr(not(windows), allow(dead_code))]
fn validate_host_manifest(raw: &str) -> Result<String, Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| vec![format!("host manifest is not valid JSON: {e}")])?;
    let mut issues = Vec::new();
    if v["name"].as_str() != Some("io.velox.downloadmanager") {
        issues.push(format!(
            "manifest name is {:?}, expected \"io.velox.downloadmanager\"",
            v["name"].as_str().unwrap_or("")
        ));
    }
    if v["type"].as_str() != Some("stdio") {
        issues.push("manifest type must be \"stdio\"".into());
    }
    let exe = v["path"].as_str().unwrap_or("").to_string();
    if exe.is_empty() {
        issues.push("manifest path is empty".into());
    } else if !std::path::Path::new(&exe).exists() {
        issues.push(format!("host executable no longer exists: {exe}"));
    }
    let want = format!("chrome-extension://{EXTENSION_ID}/");
    let origins = v["allowed_origins"].as_array().cloned().unwrap_or_default();
    if !origins
        .iter()
        .any(|o| o.as_str() == Some(want.as_str()))
    {
        issues.push(format!(
            "allowed_origins does not include the shipped extension origin ({want}) — Chrome and Edge refuse every connectNative as a forbidden origin"
        ));
    }
    if issues.is_empty() {
        Ok(exe)
    } else {
        Err(issues)
    }
}

fn host_registration_unregistered() -> HostRegistration {
    HostRegistration {
        registered: false,
        manifest_path: String::new(),
        manifest_valid: false,
        issues: vec!["no native-host registration found for any supported Chromium browser".into()],
    }
}

fn registration_from_manifest(manifest_path: &std::path::Path) -> HostRegistration {
    match std::fs::read_to_string(manifest_path) {
        Ok(raw) => match validate_host_manifest(&raw) {
            Ok(_) => HostRegistration {
                registered: true,
                manifest_path: manifest_path.display().to_string(),
                manifest_valid: true,
                issues: Vec::new(),
            },
            Err(issues) => HostRegistration {
                registered: true,
                manifest_path: manifest_path.display().to_string(),
                manifest_valid: false,
                issues,
            },
        },
        Err(e) => HostRegistration {
            registered: true,
            manifest_path: manifest_path.display().to_string(),
            manifest_valid: false,
            issues: vec![format!("registered manifest file cannot be read: {e}")],
        },
    }
}

#[cfg(windows)]
fn probe_host_registration() -> HostRegistration {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let hk = winreg::RegKey::predef(HKEY_CURRENT_USER);
    for b in HOST_REG_BROWSERS {
        let key = format!(r"Software\{b}\NativeMessagingHosts\io.velox.downloadmanager");
        let Ok(sub) = hk.open_subkey_with_flags(&key, KEY_READ) else {
            continue;
        };
        let Ok(path) = sub.get_value::<String, _>("") else {
            // key exists without a default value: Chrome/Edge treat that as a
            // broken registration — report it instead of green-lighting it
            return HostRegistration {
                registered: true,
                manifest_path: String::new(),
                manifest_valid: false,
                issues: vec![format!("registry key {key} has no manifest path (default value missing)")],
            };
        };
        return registration_from_manifest(std::path::Path::new(&path));
    }
    host_registration_unregistered()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn probe_host_registration() -> HostRegistration {
    let home = std::env::var("HOME").unwrap_or_default();
    for dir in [
        ".config/google-chrome/NativeMessagingHosts",
        ".config/google-chrome-for-testing/NativeMessagingHosts",
        ".config/microsoft-edge/NativeMessagingHosts",
        ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
        ".config/chromium/NativeMessagingHosts",
        ".config/vivaldi/NativeMessagingHosts",
    ] {
        let p = std::path::Path::new(&home).join(dir).join("io.velox.downloadmanager.json");
        if p.exists() {
            return registration_from_manifest(&p);
        }
    }
    host_registration_unregistered()
}

#[cfg(target_os = "macos")]
fn probe_host_registration() -> HostRegistration {
    host_registration_unregistered()
}

/// One-click setup for ONE browser. `mode`:
///   "guide" — stage + register + open the browser's extensions page (persistent)
///   "quick" — additionally launch the browser with --load-extension for an
///             IMMEDIATE in-session load (Chromium drops it on full restart;
///             the guided steps make it permanent).
#[tauri::command]
fn extension_setup(browser: String, mode: String) -> Result<serde_json::Value, String> {
    let browsers = detect_browsers();
    let info = browsers
        .iter()
        .find(|b| b.id == browser)
        .ok_or_else(|| format!("browser '{browser}' not detected"))?;
    if info.exe.is_empty() {
        return Err(format!("{} is not installed (or could not be located)", info.name));
    }
    if info.family != "chromium" {
        return Err("Firefox cannot be automated: it forbids programmatic extension installs. Use about:debugging → Load Temporary Add-on and pick the staged extension folder shown in the Browser screen.".into());
    }
    let src = extension_source_dir().ok_or("bundled extension assets missing (installation layout)")?;

    // 1) stage into the stable per-user location
    let dst = extension_install_dir();
    if dst.exists() {
        std::fs::remove_dir_all(&dst).map_err(|e| e.to_string())?;
    }
    copy_dir_recursive(&src, &dst)?;
    if !dst.join("manifest.json").exists() {
        return Err("staged extension is incomplete (manifest.json missing)".into());
    }

    // 2) register the native messaging host (sidecar beside the app exe)
    let mut notes: Vec<String> = Vec::new();
    let host = bundled_tool_path("velox-browser-host");
    let mut registered = match host {
        Some(h) => {
            let mut cmd = std::process::Command::new(&h);
            cmd.arg("--register").arg(EXTENSION_ID);
            #[cfg(windows)]
            {
                // console-subsystem sidecar: suppress the window flash (RC5), keep output
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let out = cmd
                .output()
                .map_err(|e| format!("native host registration failed to run: {e}"))?;
            if out.status.success() {
                notes.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
                true
            } else {
                notes.push(format!(
                    "native host registration failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
                false
            }
        }
        None => {
            notes.push("velox-browser-host sidecar not found beside Velox.exe — extension can load but will report 'Velox is not running'".into());
            false
        }
    };
    // verify the registration the way Chrome/Edge will consume it: a silent
    // --register failure (or a stale manifest) must never end as a green
    // "registered" toast (audit RC-3)
    let post = probe_host_registration();
    if registered && (!post.registered || !post.manifest_valid) {
        registered = false;
        notes.push(format!("registration did not verify: {}", post.issues.join("; ")));
    }
    if registered {
        notes.push(format!("host manifest: {}", post.manifest_path));
    }

    // 3) open the browser: extensions page (guide) or session load (quick)
    let launched = if mode == "quick" {
        std::process::Command::new(&info.exe)
            .arg(format!("--load-extension={}", dst.display()))
            .arg(&info.extensions_page)
            .spawn()
            .is_ok()
    } else {
        std::process::Command::new(&info.exe)
            .arg(&info.extensions_page)
            .spawn()
            .is_ok()
    };
    if mode == "quick" && launched {
        notes.push("temporary load: if this browser was already running, the flag is ignored — fully quit it first, or use the guided steps".into());
        notes.push("Chrome 137+ removed --load-extension from branded Chrome: if the extension does not appear, use the guided steps (Developer mode + Load unpacked)".into());
    }

    Ok(serde_json::json!({
        "ok": true,
        "browser": info.name,
        "extension_dir": dst.display().to_string(),
        "registered": registered,
        "launched": launched,
        "mode": mode,
        "extension_id": EXTENSION_ID,
        "notes": notes,
    }))
}

#[tauri::command]
fn extension_status() -> serde_json::Value {
    // bridge liveness: parse bridge.json and TCP-connect to its port
    let data_dir_bridge = std::path::Path::new(&crate_data_dir())
        .join("data")
        .join("bridge.json");
    let mut bridge_listening = false;
    let mut port: Option<u16> = None;
    if let Ok(raw) = std::fs::read_to_string(&data_dir_bridge) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            port = v["port"].as_u64().map(|p| p as u16);
        }
    }
    if let Some(p) = port {
        bridge_listening = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], p)),
            std::time::Duration::from_millis(400),
        )
        .is_ok();
    }

    let last = BRIDGE_LAST_MS.load(Ordering::Relaxed);
    let last_secs = if last == 0 {
        None
    } else {
        Some(now_ms().saturating_sub(last) / 1000)
    };
    let connected = matches!(last_secs, Some(s) if s < 120);

    // host registration probe: reads the actual registration and VALIDATES the
    // manifest content (a stale key alone no longer reports "registered")
    let reg = probe_host_registration();

    let ext_dir = extension_install_dir();
    // bundled-extractor probes: bin/ (or flat legacy) beside-exe wins, PATH
    // fallback reported as "path"
    let exe_name = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_string() };
    let probe_tool = |n: &str| -> &'static str {
        if bundled_tool_path(n).is_some() {
            "bundled"
        } else if find_on_path(&exe_name(n)).is_some() {
            "path"
        } else {
            "missing"
        }
    };
    let ytdlp = probe_tool("yt-dlp");
    let ffmpeg = probe_tool("ffmpeg");

    serde_json::json!({
        "bridge_listening": bridge_listening,
        "extension_connected": connected,
        "last_bridge_secs_ago": last_secs,
        "host_registered": reg.registered,
        "host_manifest_valid": reg.manifest_valid,
        "host_manifest_path": reg.manifest_path,
        "host_issues": reg.issues,
        "ext_installed": ext_dir.join("manifest.json").exists(),
        "ext_dir": ext_dir.display().to_string(),
        "extension_id": EXTENSION_ID,
        "host_exe_present": bundled_tool_path("velox-browser-host").is_some(),
        "ytdlp": ytdlp,
        "ffmpeg": ffmpeg,
        "browsers": detect_browsers(),
    })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    std::env::var_os("PATH")
        .map(|p| p.to_string_lossy().to_string())?
        .split(sep)
        .map(PathBuf::from)
        .map(|d| d.join(name))
        .find(|p| p.exists())
}

/// Where the per-user app data lives (mirrors the Tauri resolver; used by
/// extension_status to probe the bridge file without a State handle).
fn crate_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
            .join("io.velox.downloadmanager")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join("Library/Application Support/io.velox.downloadmanager")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("io.velox.downloadmanager")
    }
}

/// Forward engine events to the webview. The engine already broadcasts through
/// its event hub; this pump is the only bridge between the engine and the UI —
/// no polling, no invented progress.
fn spawn_event_pump(app: AppHandle, manager: Arc<DownloadManager>) {
    tauri::async_runtime::spawn(async move {
        let mut rx = manager.events();
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    // EXACT SAME WIRE CONTRACT as the HTTP preview's SSE stream:
                    // engine-bridge::wire_event defines the event names and payload
                    // shapes the frontend already understands — no second contract.
                    let (name, json) = wire_event(&ev);
                    let _ = app.emit(&name, serde_json::from_str::<serde_json::Value>(&json).unwrap_or(serde_json::Value::Null));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/* ================= browser-extension loopback bridge =================
 * docs/FEATURE_COMPARISON.md §3. The native messaging host (crates/velox-host)
 * is the only intended client: Chrome/Edge/Brave launch it, it speaks NM
 * framing on stdio, and forwards WHITELISTED JSON-RPC methods to THIS endpoint.
 * Security: loopback bind, random bearer token published only through a
 * 0600 file in app data, body caps, method whitelist, engine stays the
 * single source of truth. Ideas adapted from qamarabbas408/velox-download-manager (MIT).
 * ==================================================================== */
const BRIDGE_METHODS: &[&str] = &[
    "engine.info",
    "download.list",
    "download.status",
    "download.create",
    "download.pause",
    "download.cancel",
];
const BRIDGE_BODY_CAP: usize = 256 * 1024;
const BRIDGE_HEAD_CAP: usize = 16 * 1024;

fn bridge_token() -> String {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("os rng failure");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

async fn spawn_browser_bridge(
    data_dir: PathBuf,
    manager: Arc<DownloadManager>,
    app_version: String,
) -> Result<(), String> {
    let token = bridge_token();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let doc = serde_json::json!({
        "port": port,
        "token": token,
        "app_version": app_version,
        "pid": std::process::id(),
    });
    let path = data_dir.join("bridge.json");
    std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    eprintln!("browser bridge: 127.0.0.1:{port} (bridge.json written)");

    tauri::async_runtime::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break };
            let manager = manager.clone();
            let token = token.clone();
            // one short-lived task per connection; the host sends one request
            // per connection (Connection: close) — bounded by design
            tauri::async_runtime::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                // bounded header read
                while head.len() < BRIDGE_HEAD_CAP {
                    if stream.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                if !head.ends_with(b"\r\n\r\n") {
                    return; // oversized/malformed head
                }
                let head_str = String::from_utf8_lossy(&head);
                let auth_ok = head_str
                    .lines()
                    .any(|l| l.eq_ignore_ascii_case(&format!("authorization: bearer {token}")));
                let content_length = head_str
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if !auth_ok || content_length == 0 || content_length > BRIDGE_BODY_CAP {
                    return; // silent drop: not our client / bad request
                }
                BRIDGE_LAST_MS.store(now_ms(), Ordering::Relaxed);
                let mut body = vec![0u8; content_length];
                if stream.read_exact(&mut body).await.is_err() {
                    return;
                }
                let body_str = String::from_utf8_lossy(&body).to_string();

                // method whitelist BEFORE dispatch (the engine API itself must
                // stay unreachable for browser-driven non-whitelisted methods)
                let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap_or(serde_json::Value::Null);
                let method = parsed["method"].as_str().unwrap_or("").to_string();
                let resp = if BRIDGE_METHODS.contains(&method.as_str()) {
                    engine_api::handle_line(&manager, &body_str)
                        .await
                        .unwrap_or_else(|| {
                            serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"no response"}}).to_string()
                        })
                } else {
                    serde_json::json!({
                        "jsonrpc":"2.0","id":parsed["id"],
                        "error":{"code":-32601,"message":"method not allowed over the browser bridge"}
                    })
                    .to_string()
                };
                let http = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp.len(),
                    resp
                );
                let _ = stream.write_all(http.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        // Auto-update (v0.3.0): signed artifacts on GitHub releases; the frontend
        // drives check/downloadAndInstall/relaunch through the plugin globals.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(single_instance(|app, _args, _cwd| {
            // second launch: surface the existing window instead of starting a
            // second engine (which would fight over the SQLite database and the
            // per-download OS locks)
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            // Windows-integration paths: engine state under the per-user app data
            // dir, downloads under the user's Downloads folder — never inside the
            // installation directory.
            let data_dir = app.path().app_data_dir()?.join("data");
            let downloads_root = app.path().download_dir()?.join("Velox");
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&downloads_root).map_err(|e| e.to_string())?;

            // Bundled media tooling (offline setup ships yt-dlp/ffmpeg beside the
            // exe): prepend the install dir to PATH so the bundled binaries win
            // over whatever happens to be on PATH. The extractor config then uses
            // the absolute path when present.
            // 0.7.0+ layout: bundled dependencies live in bin/ beside the exe
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
                .unwrap_or_else(|| PathBuf::from("yt-dlp"));

            let cfg = EngineConfig {
                data_dir: data_dir.clone(),
                default_out_dir: downloads_root.clone(),
                // media-page URLs (YouTube) resolve via the optional yt-dlp extractor:
                // the BUNDLED binary beside the exe wins, else PATH lookup; absent
                // both it fails with clear guidance instead of downloading a watch
                // page (see docs/EXTRACTORS.md)
                extractor_path: Some(extractor),
                ..EngineConfig::default()
            };
            let backend = Arc::new(ReqwestBackend::new(&cfg).map_err(|e| e.to_string())?);
            let manager = DownloadManager::new(cfg, backend).map_err(|e| e.to_string())?;

            // MediaPipeline merge stage: discover an LGPL ffmpeg on PATH and inject
            // the port. Honest stderr line when absent — merging stays unavailable
            // instead of pretending (docs/FFMPEG_INTEGRATION.md §3).
            match tauri::async_runtime::block_on(engine_media::MergePort::discover_subprocess_at(tools_dir.as_deref())) {
                Ok(port) => {
                    manager.set_media_merge(Some(Arc::new(port)));
                    eprintln!("media merge: ready (LGPL ffmpeg found)");
                }
                Err(e) => eprintln!(
                    "media merge: unavailable ({}) — merging disabled (docs/FFMPEG_INTEGRATION.md §3)",
                    e.message
                ),
            }

            let settings_path = data_dir.join("ui-settings.json");
            let themes_dir = app.path().app_data_dir()?.join("themes");
            std::fs::create_dir_all(&themes_dir).map_err(|e| e.to_string())?;
            app.manage(AppState {
                manager: manager.clone(),
                downloads_root: downloads_root.clone(),
                settings_path,
                themes_dir,
                shutting_down: AtomicBool::new(false),
            });

            spawn_event_pump(app.handle().clone(), manager.clone());

            // browser-extension loopback bridge (best-effort: without it the
            // extension simply reports "Velox is not running")
            let app_version = app.package_info().version.to_string();
            if let Err(e) = tauri::async_runtime::block_on(spawn_browser_bridge(data_dir, manager, app_version)) {
                eprintln!("browser bridge: unavailable ({e})");
            }

            // window icon + title are set from tauri.conf.json; nothing else needed
            Ok(())
        })
        .on_window_event(|window, event| {
            // shutdown policy (documented in RELEASE-AND-UPDATE-GUIDE.md): the FIRST
            // close request is intercepted; active downloads are paused and the pause
            // operations are AWAITED under a bounded timeout; only then the window
            // closes and the process exits. Engine checkpoints are durable on their
            // cadence, so nothing is lost even if the bound is hit.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let Some(state) = window.app_handle().try_state::<AppState>() else {
                    return;
                };
                if state.shutting_down.swap(true, Ordering::SeqCst) {
                    // shutdown already in flight: honor the repeated close request
                    // immediately — no second pause task, no deadlock
                    return;
                }
                api.prevent_close();
                let mgr = state.manager.clone();
                let win = window.clone();
                tauri::async_runtime::spawn(async move {
                    let rows = mgr.list().unwrap_or_default();
                    // bounded shutdown: 10 s per running download, 30 s overall —
                    // the app closes even if a pause stalls
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        for row in rows {
                            if row.running {
                                let _ = tokio::time::timeout(
                                    std::time::Duration::from_secs(10),
                                    mgr.pause(row.id),
                                )
                                .await;
                            }
                        }
                    })
                    .await;
                    // pauses settled (or bounded): close for real — this re-enters
                    // CloseRequested, sees the flag, and lets the window close
                    let _ = win.close();
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
        rpc,
        default_download_dir,
        open_path,
        settings_get,
        settings_set,
        browser_detect,
        extension_setup,
        extension_status,
        themes_dir,
        themes_list,
        theme_install,
        theme_uninstall
    ])
        .run(tauri::generate_context!())
        .expect("error while running Velox");
}

#[cfg(test)]
mod tests {
    use super::{edge_exe_candidates, validate_host_manifest, HOST_REG_BROWSERS};
    use engine_core::extension::EXTENSION_ID;

    /// A manifest pointing at a REAL file (path existence is part of the check).
    fn valid_manifest() -> (String, std::path::PathBuf) {
        let exe = std::env::temp_dir().join(format!(
            "velox-host-probe-test-{}.exe",
            std::process::id()
        ));
        std::fs::write(&exe, b"mz").expect("write probe exe");
        let manifest = serde_json::json!({
            "name": "io.velox.downloadmanager",
            "description": "Velox browser integration host",
            "path": exe.display().to_string(),
            "type": "stdio",
            "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
        })
        .to_string();
        (manifest, exe)
    }

    #[test]
    fn valid_manifest_passes_and_reports_the_host_path() {
        let (raw, exe) = valid_manifest();
        let path = validate_host_manifest(&raw).expect("a complete, present manifest is valid");
        assert_eq!(std::path::Path::new(&path), exe.as_path());
        let _ = std::fs::remove_file(&exe);
    }

    #[test]
    fn every_broken_aspect_is_named_individually() {
        let dead_exe = std::env::temp_dir().join("velox-host-probe-test-definitely-gone.exe");
        let manifest = serde_json::json!({
            "name": "io.velox.downloadmanager",
            "path": dead_exe.display().to_string(),
            "allowed_origins": ["chrome-extension://hpaxoarvxcmmqijm/"],
        })
        .to_string();
        let issues = validate_host_manifest(&manifest).expect_err("dead exe + missing type + wrong origin");
        assert!(issues.iter().any(|i| i.contains("no longer exists")), "dead exe reported: {issues:?}");
        assert!(issues.iter().any(|i| i.contains("stdio")), "missing type reported: {issues:?}");
        assert!(
            issues.iter().any(|i| i.contains("allowed_origins") && i.contains(EXTENSION_ID)),
            "stale origin reported against the DERIVED id: {issues:?}"
        );
    }

    #[test]
    fn malformed_json_is_a_single_clear_issue() {
        let issues = validate_host_manifest("not json at all").expect_err("malformed manifest");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("valid JSON"));
    }

    #[test]
    fn stale_registration_shape_is_reported_not_greenlit() {
        // what an uninstalled predecessor leaves behind: a manifest whose exe
        // is gone. registered=true + manifest_valid=false is exactly the state
        // the Browser screen's Repair flow is driven by.
        let issues = validate_host_manifest(&serde_json::json!({
            "name": "io.velox.downloadmanager",
            "path": "C:\\does\\not\\exist\\velox-browser-host.exe",
            "type": "stdio",
            "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
        })
        .to_string())
        .expect_err("dead exe");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no longer exists"));
    }

    #[test]
    fn edge_discovery_includes_the_per_user_install_location() {
        let cands = edge_exe_candidates("C:\\Program Files (x86)", "C:\\Users\\u\\AppData\\Local");
        assert_eq!(cands.len(), 2, "system + per-user Edge locations");
        assert!(cands[0].starts_with("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"));
        assert!(
            cands[1].starts_with("C:\\Users\\u\\AppData\\Local\\Microsoft\\Edge\\Application\\msedge.exe"),
            "per-user Edge install (RC-4) must be probed: {cands:?}"
        );
    }

    #[test]
    fn host_registration_probe_covers_chrome_and_edge() {
        for b in ["Google\\Chrome", "Microsoft\\Edge"] {
            assert!(HOST_REG_BROWSERS.contains(&b), "missing probe key {b}");
        }
        // kept in sync with the host's own registration list
        assert_eq!(HOST_REG_BROWSERS.len(), 6);
    }
}
