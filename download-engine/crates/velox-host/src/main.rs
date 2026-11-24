//! velox-browser-host — Chrome/Edge/Brave Native Messaging host for Velox.
//!
//! Speaks Chrome's NM framing on stdio (4-byte little-endian length prefix +
//! UTF-8 JSON) and translates versioned envelope messages into calls against
//! the Velox desktop app's loopback bridge (HTTP/1.1 POST /rpc with a bearer
//! token). The DESKTOP ENGINE stays the single source of truth: this process
//! never stores download state, it only forwards requests and reports results.
//!
//! Protocol v1 envelope: {version, id, type, payload} — see docs/FEATURE_COMPARISON.md §3.
//! Security: message-size caps, malformed-frame tolerance, bridge token from a
//! 0600 file written by the app, method whitelist enforced app-side.
//!
//! Ideas adapted from qamarabbas408/velox-download-manager (MIT) — see
//! docs/THIRD_PARTY_NOTICES.md. Implementation is original against our contract.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 1;
pub const HOST_NAME: &str = "io.velox.downloadmanager";
/// The extension ID Chrome derives from the shipped manifest key. Pinned here
/// AND re-derived in tests from download-engine/extension/manifest.json
/// (audit EXT-1: the previously pinned ID did not match the key, so Chrome
/// refused every native-messaging connection as a forbidden origin).
pub const EXTENSION_ID: &str = "ffnpjmbhfmaamhlbohbmamnmoaikpdmc";
/// Chrome caps extension→host messages at 4 MiB; accept no more.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// Response body cap from the loopback bridge (engine responses are small).
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub const K_HELLO: &str = "hello";
pub const K_HELLO_ACK: &str = "hello_ack";
pub const K_PING: &str = "ping";
pub const K_PONG: &str = "pong";
pub const K_ADD_REQUEST: &str = "add_request";
pub const K_ACCEPTED: &str = "download_accepted";
pub const K_REJECTED: &str = "download_rejected";
pub const K_STATUS_REQ: &str = "status_request";
pub const K_STATUS_RSP: &str = "status_response";
pub const K_CANCEL_REQ: &str = "cancel_request";
pub const K_CANCEL_ACK: &str = "cancel_accepted";
pub const K_ERROR: &str = "error";

/* ---------------- NM framing ---------------- */

/// Read one length-prefixed frame from `r`. Ok(None) on clean EOF.
pub fn read_frame(r: &mut impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let size = u32::from_le_bytes(len) as usize;
    if size > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame of {size} bytes exceeds the {MAX_MESSAGE_BYTES}-byte limit"),
        ));
    }
    let mut buf = vec![0u8; size];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

pub fn write_frame(w: &mut impl Write, data: &[u8]) -> std::io::Result<()> {
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)?;
    w.flush()
}

pub fn envelope(id: &str, kind: &str, payload: Value) -> Value {
    json!({"version": PROTOCOL_VERSION, "id": id, "type": kind, "payload": payload})
}

pub fn error_env(id: &str, code: &str, message: &str) -> Value {
    envelope(id, K_ERROR, json!({"code": code, "message": message}))
}

/* ---------------- bridge discovery ---------------- */

/// Where the desktop app publishes {port, token, app_version, pid}.
pub fn bridge_file() -> PathBuf {
    if let Ok(p) = std::env::var("VELOX_BRIDGE_FILE") {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base)
            .join("io.velox.downloadmanager")
            .join("data")
            .join("bridge.json")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join("Library/Application Support/io.velox.downloadmanager/data/bridge.json")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local/share"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("io.velox.downloadmanager")
            .join("data")
            .join("bridge.json")
    }
}

#[derive(Debug, Clone)]
pub struct Bridge {
    pub port: u16,
    pub token: String,
    pub app_version: String,
}

/// Load + validate the bridge descriptor. `Err` carries a user-presentable reason.
pub fn load_bridge() -> Result<Bridge, String> {
    let path = bridge_file();
    let raw = std::fs::read_to_string(&path).map_err(|_| {
        "Velox is not running or the bridge file is missing — start the Velox desktop app first"
            .to_string()
    })?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("bridge file malformed: {e}"))?;
    let port = v["port"]
        .as_u64()
        .filter(|p| *p > 0 && *p < 65536)
        .ok_or("bridge port invalid")? as u16;
    let token = v["token"]
        .as_str()
        .filter(|t| t.len() >= 32)
        .ok_or("bridge token missing/short")?
        .to_string();
    let app_version = v["app_version"].as_str().unwrap_or("unknown").to_string();
    Ok(Bridge {
        port,
        token,
        app_version,
    })
}

/* ---------------- minimal loopback HTTP client ---------------- */

/// POST /rpc with the bearer token; returns the response body (bounded).
pub fn rpc_call(bridge: &Bridge, body: &str) -> Result<String, String> {
    if body.len() > 256 * 1024 {
        return Err("request body too large".into());
    }
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", bridge.port))
        .map_err(|e| format!("cannot reach the Velox bridge: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(10))))
        .map_err(|e| e.to_string())?;
    let req = format!(
        "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        bridge.token,
        body.len(),
        body
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if buf.len() + n > MAX_RESPONSE_BYTES {
            return Err("bridge response too large".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    let split = text.find("\r\n\r\n").ok_or("malformed bridge response")?;
    let head = &text[..split];
    let status_ok = head
        .lines()
        .next()
        .map(|l| l.contains(" 200 "))
        .unwrap_or(false);
    let body = text[split + 4..].to_string();
    if !status_ok {
        return Err(format!(
            "bridge returned non-200: {}",
            head.lines().next().unwrap_or("")
        ));
    }
    Ok(body)
}

/// Call a whitelisted RPC method; engine errors surface as Err(message).
pub fn rpc_method(bridge: &Bridge, method: &str, params: Value) -> Result<Value, String> {
    let body =
        json!({"jsonrpc": "2.0", "id": "host", "method": method, "params": params}).to_string();
    let raw = rpc_call(bridge, &body)?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("bridge JSON malformed: {e}"))?;
    if let Some(err) = v["error"].as_object() {
        return Err(err["message"]
            .as_str()
            .unwrap_or("engine error")
            .to_string());
    }
    Ok(v["result"].clone())
}

/* ---------------- host main loop ---------------- */

pub fn handle_message(bridge: &Option<Bridge>, env: &Value) -> Option<Value> {
    let id = env["id"].as_str().unwrap_or("0").to_string();
    let kind = env["type"].as_str().unwrap_or("");
    let version = env["version"].as_u64().unwrap_or(0);
    if version != PROTOCOL_VERSION as u64 {
        return Some(error_env(&id, "E_VERSION", "unsupported protocol version"));
    }
    match kind {
        K_HELLO => {
            let ext = env["payload"]["ext_version"].as_str().unwrap_or("unknown");
            let browser = env["payload"]["browser"].as_str().unwrap_or("chromium");
            Some(match bridge {
                Some(b) => envelope(
                    &id,
                    K_HELLO_ACK,
                    json!({"connected": true, "app_version": b.app_version, "protocol": PROTOCOL_VERSION, "browser": browser, "ext_version": ext}),
                ),
                None => envelope(
                    &id,
                    K_HELLO_ACK,
                    json!({"connected": false, "reason": "Velox is not running (bridge file missing)", "protocol": PROTOCOL_VERSION}),
                ),
            })
        }
        K_PING => Some(envelope(
            &id,
            K_PONG,
            json!({"ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)}),
        )),
        K_ADD_REQUEST => {
            // validate input BEFORE availability so malformed requests are
            // rejected honestly regardless of whether Velox is running
            let url = env["payload"]["url"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Some(envelope(
                    &id,
                    K_REJECTED,
                    json!({"code": "INVALID_URL", "message": "only http(s) URLs are accepted"}),
                ));
            }
            let Some(b) = bridge else {
                return Some(error_env(
                    &id,
                    "E_NO_APP",
                    "Velox is not running — start the desktop app and retry",
                ));
            };
            let mut params = json!({"url": url, "auto_start": true});
            if let Some(name) = env["payload"]["suggested_name"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                params["filename"] = json!(name);
            }
            if let Some(referer) = env["payload"]["referrer"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                params["referer"] = json!(referer);
            }
            match rpc_method(b, "download.create", params) {
                Ok(result) => {
                    let did = result["id"].as_i64().unwrap_or(0);
                    Some(envelope(
                        &id,
                        K_ACCEPTED,
                        json!({"download_id": did, "filename": result["final_name"], "total_bytes": result["total_size"]}),
                    ))
                }
                Err(e) => Some(envelope(
                    &id,
                    K_REJECTED,
                    json!({"code": "ENGINE", "message": e}),
                )),
            }
        }
        K_STATUS_REQ => {
            let Some(b) = bridge else {
                return Some(error_env(&id, "E_NO_APP", "Velox is not running"));
            };
            let info = rpc_method(b, "engine.info", json!({})).ok();
            let rows = match rpc_method(b, "download.list", json!({})) {
                Ok(Value::Array(rows)) => rows,
                _ => Vec::new(),
            };
            let active = rows
                .iter()
                .filter(|r| matches!(r["state"].as_str(), Some("downloading") | Some("retrying")))
                .count();
            let agg_bps: f64 = rows
                .iter()
                .filter(|r| r["state"] == "downloading")
                .filter_map(|r| r["bps_smoothed"].as_f64().or(r["bps"].as_f64()))
                .sum();
            let recent: Vec<Value> = rows
                .iter()
                .rev()
                .take(5)
                .map(|r| {
                    json!({"id": r["id"], "name": r["final_name"], "state": r["state"],
                           "received": r["received"], "total": r["total_size"]})
                })
                .collect();
            Some(envelope(
                &id,
                K_STATUS_RSP,
                json!({
                    "app_version": info.as_ref().and_then(|i| i["version"].as_str()).unwrap_or("unknown"),
                    "active": active,
                    "agg_bps": agg_bps,
                    "recent": recent,
                }),
            ))
        }
        K_CANCEL_REQ => {
            let Some(b) = bridge else {
                return Some(error_env(&id, "E_NO_APP", "Velox is not running"));
            };
            let Some(did) = env["payload"]["download_id"].as_i64() else {
                return Some(error_env(
                    &id,
                    "E_BAD_REQUEST",
                    "download_id must be numeric",
                ));
            };
            match rpc_method(b, "download.cancel", json!({"id": did})) {
                Ok(_) => Some(envelope(&id, K_CANCEL_ACK, json!({"download_id": did}))),
                Err(e) => Some(error_env(&id, "E_ENGINE", &e)),
            }
        }
        "" => Some(error_env(&id, "E_BAD_REQUEST", "missing message type")),
        other => Some(error_env(
            &id,
            "E_BAD_TYPE",
            &format!("unknown message type '{other}'"),
        )),
    }
}

pub fn serve_stdio() -> i32 {
    let bridge = load_bridge().ok();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut r = stdin.lock();
    let mut w = stdout.lock();
    loop {
        match read_frame(&mut r) {
            Ok(None) => return 0, // browser closed the pipe — graceful shutdown
            Ok(Some(bytes)) => {
                let env: Value = match serde_json::from_slice(&bytes) {
                    Ok(v) => v,
                    Err(e) => {
                        let resp =
                            error_env("0", "E_MALFORMED", &format!("unparseable message: {e}"));
                        if write_frame(&mut w, resp.to_string().as_bytes()).is_err() {
                            return 0;
                        }
                        continue;
                    }
                };
                let Some(resp) = handle_message(&bridge, &env) else {
                    continue;
                };
                if write_frame(&mut w, resp.to_string().as_bytes()).is_err() {
                    return 0;
                }
            }
            Err(_) => return 0, // bad frame length / IO error: exit; Chrome restarts us
        }
    }
}

/* ---------------- native-host manifest registration ---------------- */

/// HKCU\Software subkeys whose NativeMessagingHosts\<HOST_NAME> key receives the
/// manifest path (audit EXT-3: Opera/Chromium/Vivaldi were never registered).
/// On Linux/macOS Opera reads the Google Chrome path, so the Chrome entry
/// covers it there.
#[allow(dead_code)] // consulted by the Windows code paths; tests check it everywhere
pub const WINDOWS_BROWSER_KEYS: &[&str] = &[
    "Google\\Chrome",
    "Microsoft\\Edge",
    "BraveSoftware\\Brave-Browser",
    "Opera Software",
    "Chromium",
    "Vivaldi",
];

/// Linux config dirs holding the per-user NM host manifest.
#[allow(dead_code)]
pub const UNIX_CONFIG_DIRS: &[&str] = &[
    ".config/google-chrome/NativeMessagingHosts",
    ".config/google-chrome-for-testing/NativeMessagingHosts",
    ".config/microsoft-edge/NativeMessagingHosts",
    ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
    ".config/chromium/NativeMessagingHosts",
    ".config/vivaldi/NativeMessagingHosts",
];

/// macOS config dirs with documented locations (Edge's macOS dir name is not
/// stable across versions — deliberately not written).
#[allow(dead_code)]
pub const MAC_CONFIG_DIRS: &[&str] = &[
    "Library/Application Support/Google/Chrome/NativeMessagingHosts",
    "Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts",
];

/// Every directory that can hold the host manifest on this platform.
#[cfg(unix)]
fn manifest_dirs(home: &std::path::Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    let bases = MAC_CONFIG_DIRS;
    #[cfg(all(unix, not(target_os = "macos")))]
    let bases = UNIX_CONFIG_DIRS;
    bases.iter().map(|b| home.join(b)).collect()
}

pub fn manifest_text(shim: &std::path::Path, ext_ids: &[String]) -> String {
    let origins: Vec<String> = ext_ids
        .iter()
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();
    json!({
        "name": HOST_NAME,
        "description": "Velox browser integration host",
        "path": shim.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": origins,
    })
    .to_string()
}

fn host_exe_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "velox-browser-host.exe"
    } else {
        "velox-browser-host"
    };
    let candidate = dir.join(name);
    candidate.exists().then_some(candidate)
}

#[cfg(windows)]
fn register_windows(manifest: &str, ext_ids: &[String]) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let base = std::env::var("LOCALAPPDATA").map_err(|_| String::from("LOCALAPPDATA not set"))?;
    let dir = PathBuf::from(base).join("Velox");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let manifest_path = dir.join("native-host.json");
    std::fs::write(&manifest_path, manifest).map_err(|e| e.to_string())?;
    let _ = ext_ids;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for browser in WINDOWS_BROWSER_KEYS {
        let key = format!("Software\\{browser}\\NativeMessagingHosts\\{HOST_NAME}");
        let (sub, _) = hkcu.create_subkey(&key).map_err(|e| e.to_string())?;
        sub.set_value("", &manifest_path.to_string_lossy().to_string())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
fn register_unix(manifest: &str) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| String::from("HOME not set"))?;
    for dir in manifest_dirs(std::path::Path::new(&home)) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{HOST_NAME}.json")), manifest)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `--register [EXTENSION_ID]` — write NM host manifests for every supported
/// Chromium browser. Without an argument the canonical ID (derived from the
/// shipped manifest key) is used, so a stale literal can never be registered.
pub fn cmd_register(ext_id: &str) -> Result<(), String> {
    let shim = host_exe_path().ok_or(
        "velox-browser-host must run from its installed location (no exe found beside it)",
    )?;
    let manifest = manifest_text(&shim, &[ext_id.to_string()]);
    #[cfg(windows)]
    {
        register_windows(&manifest, &[ext_id.to_string()])?;
    }
    #[cfg(unix)]
    {
        register_unix(&manifest)?;
    }
    println!(
        "registered native host '{HOST_NAME}' for all supported Chromium browsers (extension {ext_id})"
    );
    Ok(())
}

/// `--unregister` — best-effort removal.
pub fn cmd_unregister() -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for browser in WINDOWS_BROWSER_KEYS {
            let key = format!("Software\\{browser}\\NativeMessagingHosts");
            if let Ok(sub) = hkcu.open_subkey_with_flags(&key, KEY_WRITE) {
                let _ = sub.delete_value(HOST_NAME);
            }
        }
    }
    #[cfg(unix)]
    {
        let home = std::env::var("HOME").map_err(|_| String::from("HOME not set"))?;
        for dir in manifest_dirs(std::path::Path::new(&home)) {
            let _ = std::fs::remove_file(dir.join(format!("{HOST_NAME}.json")));
        }
    }
    println!("native host registration removed");
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--register") {
        // the ID argument is optional: without it the canonical ID derived from
        // the shipped manifest key is used (callers cannot pass a stale value)
        let ext_id = args
            .iter()
            .position(|a| a == "--register")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| EXTENSION_ID.to_string());
        std::process::exit(match cmd_register(&ext_id) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("register failed: {e}");
                1
            }
        });
    }
    if args.iter().any(|a| a == "--unregister") {
        std::process::exit(match cmd_unregister() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("unregister failed: {e}");
                1
            }
        });
    }
    std::process::exit(serve_stdio());
}

#[cfg(test)]
mod tests {
    use super::manifest_text;
    use super::{EXTENSION_ID, HOST_NAME, MAC_CONFIG_DIRS, UNIX_CONFIG_DIRS, WINDOWS_BROWSER_KEYS};
    use std::path::Path;

    /// The `key` value of download-engine/extension/manifest.json.
    fn shipped_manifest_key() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extension/manifest.json");
        let s = std::fs::read_to_string(&path)
            .expect("extension/manifest.json must exist beside the workspace");
        let start = s
            .find("\"key\"")
            .expect("manifest.json must carry a key field");
        let rest = &s[start..];
        let colon = rest.find(':').expect("key field followed by a colon");
        let after = &rest[colon..];
        let q1 = after.find('"').expect("quoted key value") + 1;
        let tail = &after[q1..];
        let q2 = tail.find('"').expect("closing quote of the key value");
        tail[..q2].to_string()
    }

    /// Chrome's rule: SHA-256 over the SPKI DER, first 16 bytes, each hex nibble
    /// mapped to the letters a..p. Test-only copy (the desktop app's copy lives
    /// in engine_core::extension; both are locked to the shipped key).
    mod keyid {
        pub fn id_from_key_hex(hex: &str) -> String {
            hex.chars()
                .take(32)
                .map(|c| match c {
                    '0'..='9' => (b'a' + (c as u8 - b'0')) as char,
                    'a'..='f' => (b'k' + (c as u8 - b'a')) as char,
                    _ => unreachable!("digest hex is lowercase"),
                })
                .collect()
        }

        pub fn b64_decode(s: &str) -> Vec<u8> {
            const T: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let val = |c: u8| T.iter().position(|&t| t == c).unwrap() as u32;
            let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=' && c != b'\n').collect();
            bytes
                .chunks(4)
                .flat_map(|c| {
                    let n = c.iter().fold(0u32, |a, &b| (a << 6) | val(b));
                    match c.len() {
                        4 => vec![(n >> 16) as u8, (n >> 8) as u8, n as u8],
                        3 => vec![(n >> 10) as u8, (n >> 2) as u8],
                        2 => vec![(n >> 4) as u8],
                        _ => vec![],
                    }
                })
                .collect()
        }

        pub fn id_from_spki(der: &[u8]) -> String {
            use sha2::Digest;
            id_from_key_hex(
                &sha2::Sha256::digest(der)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
            )
        }
    }

    #[test]
    fn shipped_manifest_key_derives_the_pinned_id() {
        let der = keyid::b64_decode(&shipped_manifest_key());
        assert_eq!(der.len(), 294, "RSA-2048 SPKI is 294 bytes");
        let derived = keyid::id_from_spki(&der);
        assert_eq!(
            derived, EXTENSION_ID,
            "manifest key and pinned ID drifted apart"
        );
        assert_ne!(derived, "hpaxoarvxcmmqijm", "the old, wrong ID came back");
    }

    #[test]
    fn registration_covers_the_common_chromium_browsers() {
        for key in ["Opera Software", "Chromium", "Vivaldi", "Google\\Chrome"] {
            assert!(
                WINDOWS_BROWSER_KEYS.contains(&key),
                "missing Windows key {key}"
            );
        }
        for dir in [
            ".config/chromium/NativeMessagingHosts",
            ".config/vivaldi/NativeMessagingHosts",
            ".config/google-chrome-for-testing/NativeMessagingHosts",
        ] {
            assert!(UNIX_CONFIG_DIRS.contains(&dir), "missing Linux dir {dir}");
        }
        assert!(!MAC_CONFIG_DIRS.is_empty());
    }

    #[test]
    fn manifest_text_allowlists_the_extension_origin() {
        let m = manifest_text(
            Path::new("/opt/velox/velox-browser-host"),
            &[EXTENSION_ID.to_string()],
        );
        let v: serde_json::Value = serde_json::from_str(&m).expect("manifest is valid JSON");
        assert_eq!(v["name"], HOST_NAME);
        assert_eq!(v["path"], "/opt/velox/velox-browser-host");
        assert_eq!(
            v["allowed_origins"][0],
            format!("chrome-extension://{EXTENSION_ID}/")
        );
    }
}
