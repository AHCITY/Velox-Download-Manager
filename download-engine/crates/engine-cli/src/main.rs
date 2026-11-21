//! `velox` — the first client of the Velox engine (headless CLI).
//!
//! Talks to engine-core in-process; `velox serve` exposes the same engine over JSON-RPC v1 stdio.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use engine_core::config::{DownloadConfig, EngineConfig, HashAlgo, VerifyRequest};
use engine_core::manager::{DownloadManager, StatusView};
use engine_core::ports::HttpPort as _;
use engine_http::ReqwestBackend;

fn main() -> ExitCode {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = rt.block_on(run());
    ExitCode::from(code)
}

fn resolve_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DXDL_HOME") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(p).join("velox");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/velox");
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("velox");
    }
    PathBuf::from("velox-data")
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(std::io::stderr)
        .init();
}

fn usage() -> &'static str {
    r#"velox — Velox download engine client

USAGE:
  velox download <URL> [options]
      -o, --out <DIR>            destination directory (default: $DXDL_HOME/out)
      --name <FILE>              override output filename
      -c, --connections <N>      max connections for this download
      --limit <BPS>              bandwidth limit (bytes/sec)
      -H, --header <"K: V">      extra request header (repeatable)
      --cookie <"k=v">           request cookie (repeatable)
      --referer <URL>
      --basic <user:password>
      --bearer <TOKEN>
      --proxy <URL>              http(s):// or socks5://
      --verify <algo:hex>        sha256:... or blake3:... (verified before completion)
      --overwrite                replace an existing target file
      --insecure-tls             disable certificate verification (per-download opt-in)
      --no-preallocate           skip sparse preallocation
  velox inspect <URL>            print resource metadata without downloading
  velox list                     list downloads
  velox status <ID>              detailed status incl. segments and recent errors
  velox pause <ID> | resume <ID> | cancel <ID> | remove <ID> | verify <ID>
  velox serve                    JSON-RPC v1 over stdio
  velox serve-http [options]     API v2 + web UI (desktop preview)
      --bind <ADDR>              bind address (default 127.0.0.1)
      --port <N>                 port (default 3000)
      --ui-dir <DIR>             static UI directory
      --sample-url <URL>         demo URL exposed at /config.json
  velox cookies import <file>    import a Netscape cookies.txt into the jar
  velox cookies export [file]    export the jar as Netscape format (0600)
  velox help | version
"#
}

#[derive(Debug, Clone)]
struct Opts {
    out_dir: Option<PathBuf>,
    filename: Option<String>,
    max_connections: Option<usize>,
    limit: Option<u64>,
    min_segment: Option<u64>,
    bind: String,
    port: u16,
    ui_dir: Option<String>,
    sample_url: Option<String>,
    headers: BTreeMap<String, String>,
    cookies: BTreeMap<String, String>,
    referer: Option<String>,
    basic: Option<(String, String)>,
    bearer: Option<String>,
    proxy: Option<String>,
    verify: Option<VerifyRequest>,
    overwrite: bool,
    insecure_tls: bool,
    preallocate: bool,
    allow_html: Option<bool>,
    extractor: Option<String>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            out_dir: None,
            filename: None,
            max_connections: None,
            limit: None,
            min_segment: None,
            bind: "127.0.0.1".to_string(),
            port: 3000,
            ui_dir: None,
            sample_url: None,
            headers: BTreeMap::new(),
            cookies: BTreeMap::new(),
            referer: None,
            basic: None,
            bearer: None,
            proxy: None,
            verify: None,
            overwrite: false,
            insecure_tls: false,
            preallocate: true,
            allow_html: None,
            extractor: None,
        }
    }
}

fn parse_opts(args: &[String]) -> (Opts, Vec<String>) {
    let mut opts = Opts::default();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let next = |i: &mut usize| {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match a.as_str() {
            "-o" | "--out" => opts.out_dir = Some(PathBuf::from(next(&mut i))),
            "--name" => opts.filename = Some(next(&mut i)),
            "-c" | "--connections" => opts.max_connections = next(&mut i).parse().ok(),
            "--limit" => opts.limit = next(&mut i).parse().ok(),
            "--min-segment" => opts.min_segment = next(&mut i).parse().ok(),
            "--bind" => opts.bind = next(&mut i).to_string(),
            "--port" => opts.port = next(&mut i).parse().unwrap_or(3000),
            "--ui-dir" => opts.ui_dir = Some(next(&mut i).to_string()),
            "--sample-url" => opts.sample_url = Some(next(&mut i).to_string()),
            "-H" | "--header" => {
                let v = next(&mut i);
                if let Some((k, val)) = v.split_once(':') {
                    opts.headers
                        .insert(k.trim().to_string(), val.trim().to_string());
                }
            }
            "--cookie" => {
                let v = next(&mut i);
                if let Some((k, val)) = v.split_once('=') {
                    opts.cookies
                        .insert(k.trim().to_string(), val.trim().to_string());
                }
            }
            "--referer" => opts.referer = Some(next(&mut i)),
            "--allow-html" => opts.allow_html = Some(true),
            "--extractor" => opts.extractor = Some(next(&mut i)),
            "--basic" => {
                let v = next(&mut i);
                if let Some((u, p)) = v.split_once(':') {
                    opts.basic = Some((u.to_string(), p.to_string()));
                }
            }
            "--bearer" => opts.bearer = Some(next(&mut i)),
            "--proxy" => opts.proxy = Some(next(&mut i)),
            "--verify" => {
                let v = next(&mut i);
                if let Some((algo, hex)) = v.split_once(':') {
                    if let Some(algo) = HashAlgo::parse(algo) {
                        opts.verify = Some(VerifyRequest {
                            algo,
                            expected: hex.trim().to_string(),
                        });
                    }
                }
            }
            "--overwrite" => opts.overwrite = true,
            "--insecure-tls" => opts.insecure_tls = true,
            "--no-preallocate" => opts.preallocate = false,
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    (opts, rest)
}

fn make_config(engine: &EngineConfig, opts: &Opts) -> DownloadConfig {
    let mut cfg = DownloadConfig {
        out_dir: opts.out_dir.clone(),
        filename: opts.filename.clone(),
        headers: opts.headers.clone(),
        cookies: opts.cookies.clone(),
        referer: opts.referer.clone(),
        auth: None,
        max_connections: opts.max_connections,
        bandwidth_limit_bps: opts.limit,
        min_segment_size: opts.min_segment,
        verify: opts.verify.clone(),
        preallocate: opts.preallocate,
        overwrite: opts.overwrite,
        auto_start: false,
        resume_policy: Default::default(),
        insecure_tls: opts.insecure_tls,
        allow_html: opts.allow_html,
        media_merge: None,
    };
    if let Some((user, pass)) = &opts.basic {
        cfg.auth = Some(engine_core::config::AuthConfig::Basic {
            user: user.clone(),
            password: pass.clone(),
        });
    } else if let Some(token) = &opts.bearer {
        cfg.auth = Some(engine_core::config::AuthConfig::Bearer {
            token: token.clone(),
        });
    }
    let _ = engine;
    cfg
}

async fn run() -> u8 {
    init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{}", usage());
        return 64;
    }
    let cmd = args[0].clone();
    let (opts, rest) = parse_opts(&args[1..]);

    let data_dir = resolve_data_dir();
    // persistent cookie jar: <data>/cookies.json by default; DXDL_COOKIE_JAR overrides the
    // path; DXDL_COOKIE_JAR=off disables persistence (in-memory store only)
    let cookie_jar = match std::env::var("DXDL_COOKIE_JAR") {
        Ok(v) if v == "off" || v.is_empty() => None,
        Ok(p) => Some(PathBuf::from(p)),
        Err(_) => Some(data_dir.join("cookies.json")),
    };
    let engine = EngineConfig {
        data_dir: data_dir.clone(),
        default_out_dir: data_dir.join("out"),
        proxy: opts.proxy.clone(),
        cookie_jar_path: cookie_jar,
        extractor_path: opts.extractor.clone().map(PathBuf::from),
        ..Default::default()
    };
    let backend = match ReqwestBackend::new(&engine) {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let manager = match DownloadManager::new(engine, backend) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    match cmd.as_str() {
        "download" => cmd_download(manager, rest, opts).await,
        "inspect" => cmd_inspect(manager, rest, opts).await,
        "list" => cmd_list(&manager),
        "status" => cmd_status(&manager, rest),
        "pause" => {
            simple_id(&manager, &rest, "pause", |m, id| async move {
                m.pause(id).await
            })
            .await
        }
        "resume" => cmd_resume(manager, rest).await,
        "cancel" => {
            simple_id(&manager, &rest, "cancel", |m, id| async move {
                m.cancel(id).await
            })
            .await
        }
        "remove" => {
            simple_id(&manager, &rest, "remove", |m, id| async move {
                m.cancel(id).await
            })
            .await
        }
        "verify" => cmd_verify(&manager, rest).await,
        "serve" => cmd_serve(manager).await,
        "serve-http" => cmd_serve_http(manager, opts).await,
        "cookies" => cmd_cookies(&manager, rest).await,
        "version" | "--version" => {
            println!("velox {} (engine-api v1)", env!("CARGO_PKG_VERSION"));
            0
        }
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            0
        }
        other => {
            eprintln!("unknown command: {other}\n{}", usage());
            64
        }
    }
}

fn require_url(rest: &[String]) -> Option<String> {
    let url = rest.first()?;
    (url.starts_with("http://") || url.starts_with("https://")).then(|| url.clone())
}

fn print_added(a: &engine_core::manager::AddedDownload) {
    eprintln!(
        "[{}] {} ({} bytes{}) -> {}",
        a.id,
        a.filename,
        a.total_size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown size".into()),
        if a.resumable == "yes" {
            ", resumable"
        } else if a.resumable == "no" {
            ", non-resumable"
        } else {
            ", resumability unknown"
        },
        a.dest_dir
    );
}

async fn cmd_download(manager: Arc<DownloadManager>, rest: Vec<String>, opts: Opts) -> u8 {
    let Some(url) = require_url(&rest) else {
        eprintln!("usage: velox download <URL> [options]");
        return 64;
    };
    let cfg = make_config(manager.engine_config(), &opts);
    let added = match manager.add(&url, cfg.clone()).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if let Some(existing) = added.existing_id {
        eprintln!(
            "already registered as id {} (resuming from {} bytes)",
            existing, added.resumed_from
        );
        // fallthrough: start the existing record below
    }
    print_added(&added);
    let id = added.id;
    match manager.clone().start(id).await {
        Ok(()) => monitor_until_done(manager, id).await,
        Err(e) => {
            if e.message.contains("already completed") {
                let s = manager.status(id).ok();
                if let Some(s) = s {
                    eprintln!(
                        "already completed: {}",
                        PathBuf::from(&s.dest_dir).join(&s.final_name).display()
                    );
                    return 0;
                }
            }
            eprintln!("error: {e}");
            1
        }
    }
}

/// `velox resume <ID>` — start and stay attached until the download settles.
async fn cmd_resume(manager: Arc<DownloadManager>, rest: Vec<String>) -> u8 {
    let Some(id) = rest.first().and_then(|s| s.parse::<i64>().ok()) else {
        eprintln!("usage: velox resume <ID>");
        return 64;
    };
    if let Ok(s) = manager.status(id) {
        if s.state == "completed" {
            println!("resume: already completed (id {id})");
            return 0;
        }
    }
    if let Err(e) = manager.clone().start(id).await {
        eprintln!("error: {e}");
        return 1;
    }
    monitor_until_done(manager, id).await
}

/// Event loop: keep the process alive while the download runs; translate the outcome to an exit code.
async fn monitor_until_done(manager: Arc<DownloadManager>, id: i64) -> u8 {
    let mut events = manager.events();
    let mut last_line = String::new();
    loop {
        tokio::select! {
            // portable Ctrl+C: SIGINT on Unix, console handler on Windows — same
            // graceful pause + exit code on both platforms
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\npausing (checkpointing state)…");
                let _ = manager.pause(id).await;
                eprintln!("paused. resume with: velox resume {id}");
                return 130;
            }
            ev = events.recv() => {
                match ev {
                    Ok(engine_core::events::Event::Progress { received, total, speed_bps, connections, .. }) => {
                        let line = match total {
                            Some(t) if t > 0 => {
                                let pct = (received as f64 / t as f64 * 100.0).min(100.0);
                                let eta = if speed_bps > 1.0 {
                                    let s = ((t - received) as f64 / speed_bps) as u64;
                                    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
                                } else { "--".into() };
                                format!("\r{id}: {pct:5.1}%  {:>9}/s  ETA {eta}  conns {connections}  {received}/{t}",
                                    human_bps(speed_bps))
                            }
                            _ => format!("\r{id}: {:>9}/s  conns {connections}  {received} bytes", human_bps(speed_bps)),
                        };
                        if line != last_line {
                            let _ = write!(std::io::stderr(), "{line}");
                            let _ = std::io::stderr().flush();
                            last_line = line;
                        }
                    }
                    Ok(engine_core::events::Event::Completed { path, sha256, .. }) => {
                        eprintln!("\ncompleted: {path}");
                        if let Some(h) = sha256 {
                            eprintln!("sha256: {h}");
                        }
                        return 0;
                    }
                    Ok(engine_core::events::Event::Failed { kind, message, .. }) => {
                        eprintln!("\nfailed [{kind}]: {message}");
                        return match kind.as_str() {
                            "DISK_FULL" | "PERMISSION" => 2,
                            _ => 1,
                        };
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return 1,
                }
            }
        }
    }
}

fn human_bps(bps: f64) -> String {
    let b = bps.max(0.0);
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GiB/s", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MiB/s", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.1} KiB/s", b / 1024.0)
    } else {
        format!("{b:.0} B/s")
    }
}

async fn cmd_inspect(manager: Arc<DownloadManager>, rest: Vec<String>, opts: Opts) -> u8 {
    let Some(url) = require_url(&rest) else {
        eprintln!("usage: velox inspect <URL>");
        return 64;
    };
    let _ = manager; // inspection goes through the same transport config
    let cfg = make_config(manager.engine_config(), &opts);
    let headers = engine_core::ports::RequestHeaders::from_config(
        &cfg.headers,
        &cfg.referer,
        &cfg.cookies,
        &cfg.auth,
        cfg.insecure_tls,
    );
    let backend = ReqwestBackend::new(manager.engine_config()).expect("backend");
    match backend.inspect(&url, &headers).await {
        Ok(info) => {
            println!("final_url:        {}", info.final_url);
            for hop in &info.redirect_chain {
                println!("redirect:         {hop}");
            }
            println!("status:           {}", info.status);
            println!(
                "size:             {}",
                info.content_length
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".into())
            );
            println!(
                "accepts_ranges:   {}",
                match info.range_support {
                    engine_core::inspect::RangeSupport::Yes => "yes",
                    engine_core::inspect::RangeSupport::No => "no",
                    engine_core::inspect::RangeSupport::Unknown => "unknown",
                }
            );
            println!(
                "etag:             {} {}",
                info.etag.clone().unwrap_or_default(),
                if info.etag_is_weak {
                    "(weak)"
                } else {
                    "(strong)"
                }
            );
            println!(
                "last_modified:    {}",
                info.last_modified.clone().unwrap_or_default()
            );
            println!(
                "mime:             {}",
                info.mime.clone().unwrap_or_default()
            );
            if let Some(cd) = &info.content_disposition {
                println!("disposition:      {cd}");
            }
            if let Some(enc) = &info.content_encoding {
                println!("content_encoding: {enc}");
            }
            if let Some(w) = &info.www_authenticate {
                println!("auth_challenge:   {w}");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_list(manager: &Arc<DownloadManager>) -> u8 {
    match manager.list() {
        Ok(rows) => {
            println!(
                "{:<6} {:<10} {:>12} {:>12} {:<24} NAME",
                "ID", "STATE", "RECEIVED", "TOTAL", "UPDATED"
            );
            for r in rows {
                println!(
                    "{:<6} {:<10} {:>12} {:>12} {:<24} {}",
                    r.id,
                    r.state,
                    r.received,
                    r.total_size
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".into()),
                    r.updated_at,
                    r.final_name
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn print_status(s: &StatusView) {
    println!("id:          {}", s.id);
    println!("url:         {}", s.url);
    println!("name:        {}", s.final_name);
    println!(
        "state:       {}{}",
        s.state,
        if s.running { " (running)" } else { "" }
    );
    println!(
        "mode:        {} (ranges: {}, resumable: {})",
        s.mode, s.accept_ranges, s.resumable
    );
    println!(
        "progress:    {} / {}",
        s.received,
        s.total_size
            .map(|t| t.to_string())
            .unwrap_or_else(|| "?".into())
    );
    if s.running {
        println!("connections: {}", s.connections_active);
    }
    if let (Some(code), Some(msg)) = (&s.error_code, &s.error_message) {
        println!("last error:  [{code}] {msg}");
    }
    if let Some(h) = &s.sha256 {
        println!(
            "sha256:      {h}{}",
            if s.verified { " (verified)" } else { "" }
        );
    }
    println!("updated:     {}", s.updated_at);
    if !s.segments.is_empty() {
        println!("segments:");
        for seg in &s.segments {
            let end = seg.end.map(|e| e.to_string()).unwrap_or_else(|| "∞".into());
            println!(
                "  #{}  {}..{}  pos={}{}",
                seg.idx,
                seg.start,
                end,
                seg.pos,
                if seg.completed { " ✓" } else { "" }
            );
        }
    }
}

fn cmd_status(manager: &Arc<DownloadManager>, rest: Vec<String>) -> u8 {
    let Some(id) = rest.first().and_then(|s| s.parse::<i64>().ok()) else {
        eprintln!("usage: velox status <ID>");
        return 64;
    };
    match manager.status(id) {
        Ok(mut s) => {
            if let Ok(errs) = manager.db().errors(id) {
                for (ts, msg, class, _) in errs.into_iter().take(5) {
                    s.recent_errors.push(format!("{ts} [{class}] {msg}"));
                }
            }
            print_status(&s);
            for e in &s.recent_errors {
                println!("error:       {e}");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn simple_id<F, Fut>(manager: &Arc<DownloadManager>, rest: &[String], what: &str, f: F) -> u8
where
    F: Fn(Arc<DownloadManager>, i64) -> Fut,
    Fut: std::future::Future<Output = Result<(), engine_core::EngineError>>,
{
    let Some(id) = rest.first().and_then(|s| s.parse::<i64>().ok()) else {
        eprintln!("usage: velox {what} <ID>");
        return 64;
    };
    match f(manager.clone(), id).await {
        Ok(()) => {
            println!("{what}: ok (id {id})");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn cmd_verify(manager: &Arc<DownloadManager>, rest: Vec<String>) -> u8 {
    let Some(id) = rest.first().and_then(|s| s.parse::<i64>().ok()) else {
        eprintln!("usage: velox verify <ID>");
        return 64;
    };
    match manager.verify(id).await {
        Ok(v) => {
            println!("{} ({}): {}", v.id, v.algo, v.computed);
            match (v.expected.as_deref(), v.matches) {
                (Some(exp), Some(m)) => {
                    if m {
                        println!("matches expected {exp}: OK");
                        0
                    } else {
                        println!("MISMATCH against expected {exp}");
                        1
                    }
                }
                _ => {
                    println!("no expected checksum on record; stored for future verification");
                    0
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn cmd_serve(manager: Arc<DownloadManager>) -> u8 {
    eprintln!("velox serve: JSON-RPC v1 on stdio (one request per line)");
    let serve_fut = engine_api::serve_stdio(manager);
    tokio::select! {
        r = serve_fut => {
            if let Err(e) = r {
                eprintln!("serve error: {e}");
                return 1;
            }
            0
        }
        _ = tokio::signal::ctrl_c() => 0,
    }
}

async fn cmd_serve_http(manager: Arc<DownloadManager>, opts: Opts) -> u8 {
    let addr = format!("{}:{}", opts.bind, opts.port);
    // dual-stack by default: browsers resolve `localhost` to ::1 first, so binding
    // only 0.0.0.0 (IPv4) makes the UI unreachable-by-400 on some stacks; bind both
    // when the user asks for the wildcard
    let mut listeners = Vec::new();
    if opts.bind == "0.0.0.0" {
        if let Ok(l) =
            tokio::net::TcpListener::bind((std::net::Ipv6Addr::UNSPECIFIED, opts.port)).await
        {
            listeners.push(l); // dual-stack or v6-only — both useful
        }
    }
    match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => listeners.push(l),
        Err(e) => {
            if listeners.is_empty() {
                eprintln!("serve-http: cannot bind {addr}: {e}");
                return 1;
            }
            // IPv4 already covered by a dual-stack "::" bind; skip the duplicate
        }
    }
    if listeners.len() > 1 {
        eprintln!(
            "serve-http: listening on IPv4 and IPv6 ({addr} and [::]:{})",
            opts.port
        );
    }
    let ui = opts.ui_dir.map(std::path::PathBuf::from);
    if let Some(d) = &ui {
        eprintln!("velox serve-http: UI served from {}", d.display());
    }
    eprintln!("velox serve-http: API v2 on http://{addr} (POST /rpc, GET /events) — Ctrl-C stops");
    tokio::select! {
        r = engine_bridge::serve_http(manager, listeners, ui, opts.sample_url) => {
            if let Err(e) = r {
                eprintln!("serve-http error: {e}");
                return 1;
            }
            0
        }
        _ = tokio::signal::ctrl_c() => 0,
    }
}

async fn cmd_cookies(manager: &Arc<DownloadManager>, rest: Vec<String>) -> u8 {
    let action = rest.first().map(String::as_str).unwrap_or("");
    // the backend owns the jar; reach it through engine-http
    use engine_http::ReqwestBackend;
    let backend = match manager.http().as_any().downcast_ref::<ReqwestBackend>() {
        Some(b) => b,
        None => {
            eprintln!("cookies: engine backend does not expose a cookie store");
            return 1;
        }
    };
    match action {
        "import" => {
            let Some(path) = rest.get(1) else {
                eprintln!("usage: velox cookies import <netscape-file>");
                return 64;
            };
            match std::fs::read_to_string(path) {
                Ok(content) => match backend.cookie_store() {
                    Some(store) => match engine_http::netscape::import(store, &content) {
                        Ok(n) => {
                            backend.save_cookies();
                            println!("imported {n} cookie(s) from {path} into the jar");
                            0
                        }
                        Err(e) => {
                            eprintln!("import failed: {e}");
                            1
                        }
                    },
                    None => {
                        eprintln!("cookies: no persistent jar configured (DXDL_COOKIE_JAR)");
                        1
                    }
                },
                Err(e) => {
                    eprintln!("cannot read {path}: {e}");
                    1
                }
            }
        }
        "export" => match backend.cookie_store() {
            Some(store) => {
                let out = engine_http::netscape::export(store);
                match rest.get(1) {
                    Some(path) => match write_private(path.as_str(), out.as_bytes()) {
                        Ok(()) => {
                            println!("exported jar to {path} (0600 on Unix)");
                            0
                        }
                        Err(e) => {
                            eprintln!("cannot write {path}: {e}");
                            1
                        }
                    },
                    None => {
                        print!("{out}");
                        0
                    }
                }
            }
            None => {
                eprintln!("cookies: no persistent jar configured (DXDL_COOKIE_JAR)");
                1
            }
        },
        _ => {
            eprintln!("usage: velox cookies import <file> | velox cookies export [file]");
            64
        }
    }
}

fn write_private(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        f.write_all(bytes)?;
        Ok(())
    }
}

// keep Duration import used in non-test builds
#[allow(unused)]
fn _touch(_: Duration) {}
