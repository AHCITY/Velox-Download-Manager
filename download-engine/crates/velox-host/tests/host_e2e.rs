//! End-to-end protocol tests: drive the REAL `velox-browser-host` binary over
//! stdio with Chrome-style NM framing, against (a) no bridge (honest offline
//! answers) and (b) a fake bridge HTTP server (accepted/rejected/status flows).
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};

fn frame(msg: &Value) -> Vec<u8> {
    let data = msg.to_string().into_bytes();
    let mut out = (data.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&data);
    out
}

fn read_frame(r: &mut impl Read) -> Option<Value> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).ok()?;
    let size = u32::from_le_bytes(len) as usize;
    let mut buf = vec![0u8; size];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

struct HostProc(Child);

impl HostProc {
    fn spawn(bridge_file: &std::path::Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_velox-browser-host");
        Self(
            Command::new(bin)
                .env("VELOX_BRIDGE_FILE", bridge_file)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn host"),
        )
    }
    fn ask(&mut self, msg: &Value) -> Value {
        let stdin = self.0.stdin.as_mut().unwrap();
        stdin.write_all(&frame(msg)).unwrap();
        stdin.flush().unwrap();
        read_frame(self.0.stdout.as_mut().unwrap()).expect("host reply")
    }
}

impl Drop for HostProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_bridge_file(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
    let p = dir.join("bridge.json");
    std::fs::write(&p, contents).unwrap();
    p
}

fn missing_bridge_file(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("does-not-exist.json")
}

#[test]
fn hello_without_app_answers_honestly() {
    let dir = std::env::temp_dir().join(format!("vlxhost-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = missing_bridge_file(&dir);
    let mut host = HostProc::spawn(&f);
    let reply = host.ask(&json!({"version":1,"id":"h1","type":"hello","payload":{"ext_version":"1.0.0","browser":"chrome"}}));
    assert_eq!(reply["type"], "hello_ack");
    assert_eq!(reply["payload"]["connected"], false);
    assert!(reply["payload"]["reason"]
        .as_str()
        .unwrap()
        .contains("not running"));
}

#[test]
fn ping_pong_roundtrip() {
    let dir = std::env::temp_dir().join(format!("vlxhost-ping-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = missing_bridge_file(&dir);
    let mut host = HostProc::spawn(&f);
    let reply = host.ask(&json!({"version":1,"id":"p1","type":"ping","payload":{}}));
    assert_eq!(reply["type"], "pong");
}

#[test]
fn malformed_version_and_unknown_type_rejected() {
    let dir = std::env::temp_dir().join(format!("vlxhost-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = missing_bridge_file(&dir);
    let mut host = HostProc::spawn(&f);
    let r1 = host.ask(&json!({"version":99,"id":"v1","type":"ping","payload":{}}));
    assert_eq!(r1["type"], "error");
    assert_eq!(r1["payload"]["code"], "E_VERSION");
    let r2 = host.ask(&json!({"version":1,"id":"v2","type":"make_me_sandwich","payload":{}}));
    assert_eq!(r2["type"], "error");
    assert_eq!(r2["payload"]["code"], "E_BAD_TYPE");
    let r3 =
        host.ask(&json!({"version":1,"id":"v3","type":"add_request","payload":{"url":"ftp://x"}}));
    assert_eq!(r3["type"], "download_rejected");
    assert_eq!(r3["payload"]["code"], "INVALID_URL");
}

/// A fake bridge: answers engine.info, download.create and download.list with
/// canned JSON-RPC results; records the Authorization header it received.
fn fake_bridge() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let auth_log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = auth_log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            #[allow(clippy::while_let_loop)] // test helper: read-until-header pattern
            loop {
                let n = match s.read(&mut chunk) {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&buf);
            let auth = text
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                .unwrap_or("")
                .trim()
                .to_string();
            log.lock().unwrap().push(auth);
            let body = if text.contains("\"method\":\"engine.info\"") {
                json!({"jsonrpc":"2.0","id":"host","result":{"version":"0.4.0","api_version":2}})
            } else if text.contains("\"method\":\"download.list\"") {
                json!({"jsonrpc":"2.0","id":"host","result":[
                    {"id":1,"final_name":"movie.mkv","state":"downloading","received":100,"total_size":200,"bps_smoothed":50.0},
                    {"id":2,"final_name":"song.mp3","state":"completed","received":10,"total_size":10}
                ]})
            } else if text.contains("\"method\":\"download.create\"") {
                json!({"jsonrpc":"2.0","id":"host","result":{"id":42,"final_name":"movie.mkv","total_size":200}})
            } else {
                json!({"jsonrpc":"2.0","id":"host","error":{"code":-1,"message":"nope"}})
            };
            let body = body.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    (addr, auth_log)
}

#[test]
fn full_flow_against_fake_bridge() {
    let (addr, auth_log) = fake_bridge();
    let port = addr.port();
    let dir = std::env::temp_dir().join(format!("vlxhost-flow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = temp_bridge_file(
        &dir,
        &json!({"port": port, "token": "a".repeat(40), "app_version": "0.4.0", "pid": 1})
            .to_string(),
    );
    let mut host = HostProc::spawn(&f);

    let hello = host.ask(&json!({"version":1,"id":"h","type":"hello","payload":{"ext_version":"1.0","browser":"brave"}}));
    assert_eq!(hello["payload"]["connected"], true);
    assert_eq!(hello["payload"]["app_version"], "0.4.0");

    let added = host.ask(&json!({"version":1,"id":"a","type":"add_request","payload":{"url":"https://example.com/movie.mkv","suggested_name":"movie.mkv","referrer":"https://example.com/page"}}));
    assert_eq!(added["type"], "download_accepted");
    assert_eq!(added["payload"]["download_id"], 42);
    assert_eq!(added["payload"]["filename"], "movie.mkv");

    let status = host.ask(&json!({"version":1,"id":"s","type":"status_request","payload":{}}));
    assert_eq!(status["type"], "status_response");
    assert_eq!(status["payload"]["active"], 1);
    assert_eq!(status["payload"]["agg_bps"], 50.0);
    assert_eq!(status["payload"]["recent"].as_array().unwrap().len(), 2);

    // every request carried the bearer token from the bridge file
    let log = auth_log.lock().unwrap();
    let want = format!("Authorization: Bearer {}", "a".repeat(40));
    assert!(log.iter().all(|a| *a == want), "auth headers: {log:?}");
}

#[test]
fn bridge_rejects_short_token() {
    let dir = std::env::temp_dir().join(format!("vlxhost-token-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = temp_bridge_file(
        &dir,
        &json!({"port": 1, "token": "short", "app_version": "0.4.0"}).to_string(),
    );
    let mut host = HostProc::spawn(&f);
    let reply = host
        .ask(&json!({"version":1,"id":"a","type":"add_request","payload":{"url":"https://x/y"}}));
    assert_eq!(reply["type"], "error"); // bridge load fails -> E_NO_APP style error
    assert_eq!(reply["payload"]["code"], "E_NO_APP");
}
