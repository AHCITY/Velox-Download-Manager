// Velox desktop application — Tauri 2 shell around the EXISTING Rust engine.
// The engine runs IN-PROCESS: no localhost HTTP service, no sidecar process,
// no second downloader. The browser preview (`velox serve-http`) remains a
// separate, opt-in transport for CLI/testing/remote use.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use engine_bridge::wire_event;
use engine_core::config::EngineConfig;
use engine_core::manager::DownloadManager;
use engine_http::ReqwestBackend;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_single_instance::init as single_instance;

/// Managed engine handle shared by all commands.
struct AppState {
    manager: Arc<DownloadManager>,
    downloads_root: PathBuf,
    /// Persisted UI preferences (JSON; written by settings_set).
    settings_path: PathBuf,
    /// Set by the first CloseRequested; makes repeated close requests safe.
    shutting_down: AtomicBool,
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
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&resolved)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&resolved)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&resolved)
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

            let cfg = EngineConfig {
                data_dir: data_dir.clone(),
                default_out_dir: downloads_root.clone(),
                // media-page URLs (YouTube) resolve via the optional yt-dlp extractor
                // found on PATH; absent it fails with clear guidance instead of
                // downloading a watch page (see docs/EXTRACTORS.md)
                extractor_path: Some(std::path::PathBuf::from("yt-dlp")),
                ..EngineConfig::default()
            };
            let backend = Arc::new(ReqwestBackend::new(&cfg).map_err(|e| e.to_string())?);
            let manager = DownloadManager::new(cfg, backend).map_err(|e| e.to_string())?;

            // MediaPipeline merge stage: discover an LGPL ffmpeg on PATH and inject
            // the port. Honest stderr line when absent — merging stays unavailable
            // instead of pretending (docs/FFMPEG_INTEGRATION.md §3).
            match tauri::async_runtime::block_on(engine_media::MergePort::discover_subprocess()) {
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
            app.manage(AppState {
                manager: manager.clone(),
                downloads_root: downloads_root.clone(),
                settings_path,
                shutting_down: AtomicBool::new(false),
            });

            spawn_event_pump(app.handle().clone(), manager);

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
        settings_set
    ])
        .run(tauri::generate_context!())
        .expect("error while running Velox");
}
