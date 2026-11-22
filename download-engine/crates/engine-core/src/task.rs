//! Per-download orchestration: inspect → plan/reconcile → workers → checkpoints → finalize.
//!
//! Durability ladder (ADR-0007): fsync file data before every DB commit; completion =
//! hash (optional) → sync_all → rename → dir fsync → `completed` commit.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

use engine_state::{states, Db, DownloadRow, ErrorRow, SegmentRow};

use crate::config::{DownloadConfig, EngineConfig, HashAlgo};
use crate::error::{from_io, from_status, EngineError, ErrorClass, ErrorKind};
use crate::events::Event;
use crate::inspect::{RangeSupport, ResourceInfo};
use crate::plan::{build_plan, DownloadMode, Plan};
use crate::ports::{HttpPort, RangeRequest, RequestHeaders};
use crate::retry::RetryPolicy;
use crate::segments::{SegInit, SegmentTable};
use crate::speed::BandwidthLimiter;
use engine_storage::PartFile;

pub type DownloadId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCommand {
    Run,
    Pause,
    Cancel,
}

impl TaskCommand {
    pub fn is_stop(self) -> bool {
        !matches!(self, TaskCommand::Run)
    }
}

#[derive(Debug)]
pub enum TaskOutcome {
    Completed {
        path: PathBuf,
        size: u64,
        sha256: Option<String>,
        blake3: Option<String>,
    },
    Paused,
    Cancelled,
    Failed(EngineError),
}

/// Inputs the manager assembles; the task owns everything for one download run.
pub struct TaskArgs {
    pub id: DownloadId,
    pub engine: EngineConfig,
    pub dl: DownloadConfig,
    pub http: Arc<dyn HttpPort>,
    pub db: Arc<Db>,
    pub events: tokio::sync::broadcast::Sender<Event>,
    /// Per-host connection cap shared across downloads.
    pub host_permits: Arc<Semaphore>,
    /// Final resolved URL (row.resolved_url else row.url).
    pub url: String,
    pub dest_dir: PathBuf,
    /// Part file, opened (and preserved) by the manager before the task starts.
    pub file: Arc<PartFile>,
    /// Manager-visible live segment table for status queries.
    pub live_table: Arc<std::sync::Mutex<Option<Arc<SegmentTable>>>>,
}

// ResourceInfo (Restart.new_info) dwarfs EngineError: boxed at the monitor boundary
// already; boxing here would churn every send site for no measurable gain.
#[allow(clippy::large_enum_variant)]
enum Ctrl {
    Failed(EngineError),
    Restart {
        reason: String,
        new_info: Option<ResourceInfo>,
    },
}

#[allow(dead_code)] // dl/events stay with the shared context for near-term worker-side use
struct Shared {
    id: DownloadId,
    engine: EngineConfig,
    dl: DownloadConfig,
    http: Arc<dyn HttpPort>,
    db: Arc<Db>,
    file: Arc<PartFile>,
    table: Arc<SegmentTable>,
    limiter: Option<Arc<BandwidthLimiter>>,
    permits: Arc<Semaphore>,
    headers: Arc<RequestHeaders>,
    url: String,
    if_range: Option<String>,
    /// Probe-confirmed range support for this run: workers send plain GETs when false.
    ranges_supported: bool,
    /// Page policy (P0): false → the worker validating the first body chunk rejects HTML.
    html_allowed: bool,
    events: tokio::sync::broadcast::Sender<Event>,
    ctrl: mpsc::Sender<Ctrl>,
    cmd: watch::Receiver<TaskCommand>,
}

pub fn compute_hash_blocking(path: &std::path::Path, algo: HashAlgo) -> std::io::Result<String> {
    use sha2::Digest as _;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1024 * 1024];
    match algo {
        HashAlgo::Sha256 => {
            let mut h = sha2::Sha256::new();
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex(&h.finalize()))
        }
        HashAlgo::Blake3 => {
            let mut h = blake3::Hasher::new();
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex(h.finalize().as_bytes()))
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn row_to_seg_inits(segs: &[SegmentRow]) -> Vec<SegInit> {
    segs.iter()
        .map(|s| SegInit {
            start: s.start_off.max(0) as u64,
            end: s.end_off.map(|e| e as u64),
            pos: s.pos.max(0) as u64,
            completed: s.completed,
        })
        .collect()
}

fn segs_to_rows(segs: &[crate::segments::SegSnapshot]) -> Vec<SegmentRow> {
    segs.iter()
        .map(|s| SegmentRow {
            idx: s.idx,
            start_off: s.start as i64,
            end_off: s.end.map(|e| e as i64),
            pos: s.pos as i64,
            completed: s.completed,
        })
        .collect()
}

/// Stop requested during the probe phase: settle exactly like the transfer-phase
/// stop does — persist the row state and emit the event — so manager.pause()/
/// cancel() observe a settled download instead of a stuck one (audit P1).
async fn probe_stop_settle(args: &TaskArgs, row: &mut DownloadRow, c: TaskCommand) -> TaskOutcome {
    row.state = match c {
        TaskCommand::Cancel => states::CANCELLED.to_string(),
        _ => states::PAUSED.to_string(),
    };
    args.db.update_download(row).ok();
    args.events
        .send(Event::DownloadState {
            id: args.id,
            state: row.state.clone(),
        })
        .ok();
    match c {
        TaskCommand::Cancel => TaskOutcome::Cancelled,
        _ => TaskOutcome::Paused,
    }
}

/// Main task entry: runs until Completed / Paused / Cancelled / Failed.
pub async fn run_task(args: TaskArgs, mgr_cmd: watch::Receiver<TaskCommand>) -> TaskOutcome {
    let retry = args.engine.retry.clone();
    let mut restarts = 0u32;
    let mut pending_new_info: Option<ResourceInfo> = None;
    // at most one expired-URL re-resolve per task run (bounded retry, task-scoped)
    let mut re_resolved = false;

    'restart: loop {
        // ---------- manifest row ----------
        let mut row = match args.db.get_download(args.id) {
            Ok(Some(r)) => r,
            _ => {
                return TaskOutcome::Failed(EngineError::new(
                    ErrorKind::InvalidConfig,
                    "manifest row missing",
                ))
            }
        };

        // ---------- inspect (conditional probe when resuming) ----------
        let mut probe_headers = RequestHeaders::from_config(
            &args.dl.headers,
            &args.dl.referer,
            &args.dl.cookies,
            &args.dl.auth,
            args.dl.insecure_tls,
        );
        // audit D3: the resolved URL (row.resolved_url — a Drive confirm hop or an
        // extractor-resolved media stream) is frequently cross-origin relative to the
        // URL the user typed. Explicit credentials belong to the ORIGINAL site: scope
        // them here so neither the probe nor the segment workers forward cookies/auth
        // to a page-controlled host. Same-origin rows are unaffected.
        probe_headers = probe_headers.scoped_credentials(&row.url, &args.url);
        let resuming_manifest = row.received > 0;
        if resuming_manifest {
            probe_headers.if_range = row
                .etag
                .as_deref()
                .filter(|_| !row.etag_is_weak)
                .map(str::to_string);
        }
        // stop-aware probe (audit P1): pause/cancel must settle during resolve
        // retries instead of parking the task beyond the manager's settle deadlines
        // (the probe previously ignored the command watch entirely)
        let mut stop = mgr_cmd.clone();
        let mut probe_attempts = 0u32;
        // expired resolved URL (audit P1-2): media stream URLs (googlevideo etc.)
        // expire after hours — a 403 against a RESOLVED url means the extractor
        // must re-resolve the ORIGINAL page once, not that the download is dead.
        // Clearing row.resolved_url makes the next iteration probe row.url, which
        // re-runs classification and (for media pages) the yt-dlp resolve.
        // Bounded: `re_resolved` lives at run-task scope, one re-resolve max.
        let info = loop {
            let probe_target = row.resolved_url.clone().unwrap_or_else(|| row.url.clone());
            let res = tokio::select! {
                r = args.http.resolve(&probe_target, &probe_headers) => r,
                _ = stop.changed() => {
                    let c = *stop.borrow_and_update();
                    if c.is_stop() {
                        return probe_stop_settle(&args, &mut row, c).await;
                    }
                    continue;
                }
            };
            match res {
                Ok(i) if crate::inspect::probe_usable(&i) => break i,
                Ok(i) => {
                    let e = from_status(i.status, "unexpected status during inspection");
                    if !retry.exhausted(probe_attempts) && e.classify() == ErrorClass::Temporary {
                        probe_attempts += 1;
                        let wait = retry.delay_for(probe_attempts - 1, e.retry_after);
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = stop.changed() => {
                                let c = *stop.borrow_and_update();
                                if c.is_stop() {
                                    return probe_stop_settle(&args, &mut row, c).await;
                                }
                            }
                        }
                    } else if !re_resolved
                        && row.resolved_url.is_some()
                        && matches!(e.kind, ErrorKind::Http(403))
                    {
                        re_resolved = true;
                        row.resolved_url = None;
                        args.db.update_download(&row).ok();
                        continue 'restart;
                    } else {
                        return finish_failed(&args, &mut row, e).await;
                    }
                }
                Err(e) => {
                    if !retry.exhausted(probe_attempts) && e.classify() == ErrorClass::Temporary {
                        probe_attempts += 1;
                        let wait = retry.delay_for(probe_attempts - 1, e.retry_after);
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = stop.changed() => {
                                let c = *stop.borrow_and_update();
                                if c.is_stop() {
                                    return probe_stop_settle(&args, &mut row, c).await;
                                }
                            }
                        }
                    } else if !re_resolved
                        && row.resolved_url.is_some()
                        && matches!(e.kind, ErrorKind::Http(403))
                    {
                        re_resolved = true;
                        row.resolved_url = None;
                        args.db.update_download(&row).ok();
                        continue 'restart;
                    } else {
                        return finish_failed(&args, &mut row, e).await;
                    }
                }
            }
        };

        // P0 gate: HTTP 200 is not proof of a file. A hypertext body (watch page, Drive
        // interstitial, error page) or a compressed transfer coding must fail BEFORE any
        // byte is written — never silently become a "completed download".
        if let Err(e) = page_gate(&info, &args.dl) {
            return finish_failed(&args, &mut row, e).await;
        }
        let html_allowed = html_intended(&info, &args.dl);

        if let Some(ni) = pending_new_info.take() {
            apply_new_info(&mut row, &ni);
        }

        // ---------- fresh vs resume vs restart-from-zero ----------
        // All restart triggers must set restart_zero so the stored segment set below is
        // discarded — resetting row.received alone would still reconcile old positions
        // and splice new bytes over them (found by the H3 regression test).
        let mut restart_zero = false;
        let mut restart_reason = "";
        if resuming_manifest && !check_identity(&row, &info, probe_headers.if_range.is_some()) {
            restart_zero = true;
            restart_reason = "resource changed";
        } else if resuming_manifest
            && row.received > 0
            && matches!(
                (row.total_size, info.content_length),
                (Some(old), Some(new)) if old != new as i64
            )
        {
            // validator-less resume cannot detect a replaced resource via ETag/Last-Modified
            // (audit H3); a changed total length is still hard evidence the resource changed.
            restart_zero = true;
            restart_reason = "resource length changed (validator-less resume)";
        } else if resuming_manifest
            && row.received > 0
            && matches!(info.range_support, RangeSupport::No | RangeSupport::Unknown)
        {
            // mid-file resume without range support is impossible: a plain GET body starts at 0
            restart_zero = true;
            restart_reason = "no range support; partial file unusable";
        }
        if restart_zero {
            restarts += 1;
            if restarts > 3 {
                let e = EngineError::new(
                    ErrorKind::ResourceChanged,
                    format!("{restart_reason}; giving up after 3 restarts"),
                );
                return finish_failed(&args, &mut row, e).await;
            }
            warn!(
                id = args.id,
                reason = restart_reason,
                "restarting download from zero"
            );
            args.events
                .send(Event::Restarting {
                    id: args.id,
                    reason: restart_reason.to_string(),
                })
                .ok();
            row.received = 0;
        }

        let plan = build_plan(&info, &args.engine, &args.dl);
        apply_new_info(&mut row, &info);
        row.mode = match plan.mode {
            DownloadMode::Segmented => "segmented",
            DownloadMode::Single => "single",
            DownloadMode::Stream => "stream",
        }
        .to_string();
        row.accept_ranges = match info.range_support {
            RangeSupport::Yes => "yes",
            RangeSupport::No => "no",
            RangeSupport::Unknown => "unknown",
        }
        .to_string();
        row.total_size = plan.total.map(|t| t as i64);
        row.max_connections = plan.max_connections as i64;
        row.state = states::DOWNLOADING.to_string();
        row.error_code = None;
        row.error_message = None;

        // ---------- segments: manifest reconciliation or fresh plan ----------
        let (inits, clean_start) = if resuming_manifest && !restart_zero {
            let stored = args.db.segments(args.id).unwrap_or_default();
            if stored.is_empty() {
                (plan_inits(&plan), true)
            } else {
                match reconcile(&stored, args.file.len().unwrap_or(0), plan.total) {
                    Ok(segs) => (row_to_seg_inits(&segs), false),
                    Err(reason) => {
                        restarts += 1;
                        if restarts > 3 {
                            let e = EngineError::new(ErrorKind::ResourceChanged, reason);
                            return finish_failed(&args, &mut row, e).await;
                        }
                        warn!(
                            id = args.id,
                            reason, "local file reconciliation failed; restarting"
                        );
                        args.events
                            .send(Event::Restarting {
                                id: args.id,
                                reason,
                            })
                            .ok();
                        row.received = 0;
                        (plan_inits(&plan), true)
                    }
                }
            }
        } else {
            (plan_inits(&plan), true)
        };

        // ---------- prepare part file ----------
        if clean_start {
            match plan.total {
                Some(t) if args.dl.preallocate => {
                    if let Err(e) = args.file.preallocate(t).map_err(|e| from_io(&e)) {
                        // honest, non-fatal: the write path catches real space or
                        // permission problems (DiskFull/Permission → paused,
                        // resumable) instead of failing a download that might fit
                        warn!(id = args.id, error = %e, "preallocate failed; continuing sparse");
                    }
                }
                _ => {
                    if let Err(e) = args.file.truncate_to(0).map_err(|e| from_io(&e)) {
                        warn!(id = args.id, error = %e, "truncate-to-0 failed; finalize truncates to total");
                    }
                }
            }
        }

        let table = Arc::new(SegmentTable::new(inits, args.engine.min_segment_size));
        *args.live_table.lock().unwrap() = Some(table.clone());
        args.db
            .set_segments(args.id, &segs_to_rows(&table.snapshot()))
            .ok();
        args.db.update_download(&row).ok();
        args.events
            .send(Event::DownloadState {
                id: args.id,
                state: states::DOWNLOADING.into(),
            })
            .ok();

        // ---------- shared worker context + local command watch ----------
        let (local_tx, local_rx) = watch::channel(TaskCommand::Run);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<Ctrl>(16);
        let limiter = args
            .dl
            .bandwidth_limit_bps
            .map(BandwidthLimiter::new)
            .map(Arc::new);
        let sh = Arc::new(Shared {
            id: args.id,
            engine: args.engine.clone(),
            dl: args.dl.clone(),
            http: args.http.clone(),
            db: args.db.clone(),
            file: args.file.clone(),
            table: table.clone(),
            limiter,
            permits: args.host_permits.clone(),
            headers: Arc::new(probe_headers),
            url: info.final_url.clone(),
            if_range: row
                .etag
                .as_deref()
                .filter(|_| !row.etag_is_weak)
                .map(str::to_string),
            ranges_supported: info.range_support == RangeSupport::Yes,
            html_allowed,
            events: args.events.clone(),
            ctrl: ctrl_tx,
            cmd: local_rx.clone(),
        });

        // forward manager commands to the local watch; workers see only the local one
        let fwd_cmd = local_tx.clone();
        let mut fwd_rx = mgr_cmd.clone();
        let forwarder = tokio::spawn(async move {
            loop {
                match fwd_rx.changed().await {
                    Ok(_) => {
                        let c = *fwd_rx.borrow_and_update();
                        let _ = fwd_cmd.send(c);
                        if c.is_stop() {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = fwd_cmd.send(TaskCommand::Pause);
                        return;
                    }
                }
            }
        });

        // ---------- workers ----------
        let n_workers = plan.segments.len().max(1);
        let mut joinset: JoinSet<()> = JoinSet::new();
        for wid in 0..n_workers as u64 {
            joinset.spawn(worker_loop(sh.clone(), wid));
        }

        // ---------- monitor loop ----------
        let mut monitor = tokio::time::interval(Duration::from_millis(500));
        monitor.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_received = 0u64;
        let mut speed = 0f64;
        let mut last_tick = std::time::Instant::now();
        enum Sig {
            WorkersExited,
            Ctrl(Box<Ctrl>), // boxed: Ctrl dwarfs the unit variants
            Tick,
        }
        let outcome: TaskOutcome = 'monitor: loop {
            let sig = tokio::select! {
                res = joinset.join_next() => match res {
                    None => Sig::WorkersExited,
                    Some(Ok(())) => continue 'monitor,
                    Some(Err(e)) => {
                        // a panicked worker leaves its segment owned forever → the remaining
                        // workers park until a manual pause (audit: join errors were swallowed)
                        let msg = if e.is_panic() { "worker panicked" } else { "worker task failed" };
                        let _ = sh.ctrl.send(Ctrl::Failed(EngineError::new(ErrorKind::Io, msg.to_string()))).await;
                        continue 'monitor;
                    }
                },
                Some(msg) = ctrl_rx.recv() => Sig::Ctrl(Box::new(msg)),
                _ = monitor.tick() => Sig::Tick,
            };
            match sig {
                Sig::Tick => {
                    let stats = table.stats();
                    let now = std::time::Instant::now();
                    let dt = now.duration_since(last_tick).as_secs_f64().max(0.001);
                    last_tick = now;
                    let inst = (stats.received.saturating_sub(last_received)) as f64 / dt;
                    speed = if speed == 0.0 {
                        inst
                    } else {
                        speed * 0.7 + inst * 0.3
                    };
                    last_received = stats.received;
                    args.events
                        .send(Event::Progress {
                            id: args.id,
                            received: stats.received,
                            total: plan.total,
                            speed_bps: speed,
                            connections: stats.active,
                        })
                        .ok();
                    debug!(
                        id = args.id,
                        received = stats.received,
                        active = stats.active,
                        speed_bps = speed
                    );
                    if table.has_dirty() {
                        persist_final_checkpoint(&sh, &mut row).await.ok();
                    }
                }
                Sig::Ctrl(msg) => match *msg {
                    Ctrl::Failed(e) => {
                        break on_failed(&args, &mut row, &sh, &local_tx, joinset, e).await;
                    }
                    Ctrl::Restart { reason, new_info } => {
                        if let Some(o) = maybe_restart(
                            &args,
                            &mut row,
                            &local_tx,
                            joinset,
                            reason,
                            &mut restarts,
                        )
                        .await
                        {
                            break o;
                        }
                        pending_new_info = new_info;
                        forwarder.abort();
                        continue 'restart;
                    }
                },
                Sig::WorkersExited => {
                    // last worker gone: drain a possibly-racing control message first
                    match ctrl_rx.try_recv() {
                        Ok(Ctrl::Failed(e)) => {
                            break on_failed(&args, &mut row, &sh, &local_tx, joinset, e).await;
                        }
                        Ok(Ctrl::Restart { reason, new_info }) => {
                            if let Some(o) = maybe_restart(
                                &args,
                                &mut row,
                                &local_tx,
                                joinset,
                                reason,
                                &mut restarts,
                            )
                            .await
                            {
                                break o;
                            }
                            pending_new_info = new_info;
                            forwarder.abort();
                            continue 'restart;
                        }
                        _ => break settle(sh.clone(), &mut row, &args).await,
                    }
                }
            }
        };

        forwarder.abort();
        return outcome;
    }
}

fn plan_inits(plan: &Plan) -> Vec<SegInit> {
    plan.segments
        .iter()
        .map(|s| SegInit {
            start: s.start,
            end: s.end,
            pos: 0,
            completed: false,
        })
        .collect()
}

fn apply_new_info(row: &mut DownloadRow, info: &ResourceInfo) {
    row.resolved_url = Some(info.final_url.clone());
    if info.etag.is_some() {
        row.etag = info.etag.clone();
        row.etag_is_weak = info.etag_is_weak;
    }
    if info.last_modified.is_some() {
        row.last_modified = info.last_modified.clone();
    }
    row.mime = info.mime.clone();
    row.content_encoding = info.content_encoding.clone();
}

/// P0 content policy (see page_gate): does the download context show genuine intent to
/// store an HTML document? True when the user asked for HTML explicitly (allow_html),
/// named the target *.html/.htm, or the resolved URL path is itself an .html/.htm page.
pub fn html_intended(info: &ResourceInfo, dl: &DownloadConfig) -> bool {
    if dl.allow_html == Some(true) {
        return true;
    }
    if let Some(f) = dl
        .filename
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
    {
        if f.ends_with(".html") || f.ends_with(".htm") {
            return true;
        }
    }
    if let Ok(u) = url::Url::parse(&info.final_url) {
        let p = u.path().to_ascii_lowercase();
        if p.ends_with(".html") || p.ends_with(".htm") {
            return true;
        }
    }
    false
}

/// P0 correctness gate, applied at inspect time and backed by a worker-level body check:
///
/// 1. HTTP status/metadata alone is never proof of a file. A confident HTML sniff on the
///    body fails the download with NOT_A_FILE unless `html_intended` says otherwise.
/// 2. The engine always requests `Accept-Encoding: identity`; a server that answers with
///    a compressed transfer coding would have every stored byte be encoded content.
///    That is refused loudly instead of corrupting the file.
pub fn page_gate(info: &ResourceInfo, dl: &DownloadConfig) -> Result<(), EngineError> {
    if let Some(enc) = info.content_encoding.as_deref().map(str::trim) {
        if !enc.is_empty() && !enc.eq_ignore_ascii_case("identity") {
            return Err(EngineError::new(
                ErrorKind::InvalidResponse,
                format!(
                    "server sent a compressed transfer (Content-Encoding: {enc}) although the \
                     client requested identity; storing encoded bytes would corrupt the file"
                ),
            ));
        }
    }
    if info.sniffed != crate::inspect::SniffedKind::Html {
        return Ok(());
    }
    if html_intended(info, dl) {
        return Ok(());
    }
    Err(EngineError::new(
        ErrorKind::NotAFile,
        "the server returned a web page (HTML), not the requested file — this URL is not a \
         direct download. YouTube and Google Drive page links are handled by URL resolution \
         or the optional extractor; to store a page deliberately, name the target *.html \
         or set allow_html=true",
    ))
}

/// Resume identity check (RECOVERY_DESIGN §5.1/§5.3). If-Range was answered 206 ⇒ unchanged
/// (RFC 9110 §13.1.5: a failed If-Range must produce 200).
fn check_identity(row: &DownloadRow, info: &ResourceInfo, if_range_sent: bool) -> bool {
    if row.received == 0 {
        return true;
    }
    if if_range_sent {
        return info.status == 206;
    }
    let stored_any = row.etag.is_some() || row.last_modified.is_some();
    if !stored_any {
        return true; // nothing to compare: allowed, flagged unverified in status
    }
    if let Some(et) = &row.etag {
        return info.etag.as_deref() == Some(et.as_str());
    }
    if let Some(lm) = &row.last_modified {
        return info.last_modified.as_deref() == Some(lm.as_str());
    }
    false
}

/// Manifest-vs-disk reconciliation (RECOVERY_DESIGN §5.2).
fn reconcile(
    stored: &[SegmentRow],
    file_len: u64,
    total: Option<u64>,
) -> Result<Vec<SegmentRow>, String> {
    let mut segs: Vec<SegmentRow> = stored.to_vec();
    if let Some(t) = total {
        if file_len > t {
            return Err("part file longer than the resource; contents untrustworthy".into());
        }
    }
    let max_written = segs
        .iter()
        .filter(|s| !s.completed)
        .map(|s| s.start_off + s.pos)
        .max();
    if let Some(maxw) = max_written {
        let lost = maxw > file_len as i64;
        if lost {
            let completed_beyond = segs.iter().any(|s| {
                s.completed && s.end_off.map(|e| e + 1 > file_len as i64).unwrap_or(false)
            });
            if completed_beyond {
                return Err("durable data lost beyond file end (completed segment affected); restart required".into());
            }
            for s in segs.iter_mut() {
                let avail = (file_len as i64 - s.start_off).max(0);
                if s.pos > avail {
                    s.pos = avail; // re-download the remainder; positional writes overwrite residue
                }
            }
        }
    }
    Ok(segs)
}

async fn worker_loop(sh: Arc<Shared>, wid: u64) {
    let retry = sh.engine.retry.clone();
    let mut cmd = sh.cmd.clone();
    'outer: loop {
        if cmd.borrow_and_update().is_stop() {
            break;
        }
        let work = match sh.table.take(wid) {
            Some(w) => w,
            None => {
                if sh.table.all_done() {
                    break;
                }
                tokio::select! {
                    _ = sh.table.wait_change() => continue 'outer,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => continue 'outer,
                    _ = cmd.changed() => continue 'outer,
                }
            }
        };
        // stop-aware permit wait: a worker parked on a saturated per-host semaphore
        // used to ignore Pause/Cancel until a permit freed up (pause timed out, cancel
        // reported "still settling" — user-visible hang under host contention)
        let _permit = tokio::select! {
            p = sh.permits.clone().acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => break,
            },
            _ = cmd.changed() => break 'outer, // stop requested while parked
        };
        match run_segment(&sh, work, &retry).await {
            Ok(()) => {}
            Err(e) => match *e {
                SegFailure::Stop => break 'outer,
                SegFailure::Fatal(e) => {
                    sh.ctrl.send(Ctrl::Failed(e)).await.ok();
                    break 'outer;
                }
                SegFailure::Restart { reason, new_info } => {
                    sh.ctrl.send(Ctrl::Restart { reason, new_info }).await.ok();
                    break 'outer;
                }
            },
        }
    }
}

// same size-difference rationale as Ctrl (Fatal payload vs unit variant); callers
// pass SegFailure itself in a Box
#[allow(clippy::large_enum_variant)]
enum SegFailure {
    Stop,
    Fatal(EngineError),
    Restart {
        reason: String,
        new_info: Option<ResourceInfo>,
    },
}

async fn run_segment(
    sh: &Arc<Shared>,
    work: crate::segments::Work,
    retry: &RetryPolicy,
) -> Result<(), Box<SegFailure>> {
    let mut attempt = 0u32;
    let mut cmd = sh.cmd.clone();
    'request: loop {
        if cmd.borrow_and_update().is_stop() {
            sh.table.abandon(work.idx);
            return Err(Box::new(SegFailure::Stop));
        }
        let start = sh.table.abs_pos(work.idx);
        let end = sh.table.end_of(work.idx); // live: a stolen/shrunk segment never over-requests
                                             // defensive: fully written but unmarked (abandon raced the exact boundary)
        if let Some(e) = end {
            if start > e {
                sh.table.complete(work.idx);
                return Ok(());
            }
        }
        // RFC 9110 §14.2: a server MAY ignore Range. When the probe proved ranges unsupported
        // and nothing is written yet, a plain GET is the correct request form.
        let plain_get = !sh.ranges_supported && start == 0;
        let req = RangeRequest {
            url: sh.url.clone(),
            start,
            end,
            if_range: sh.if_range.clone(),
            headers: (*sh.headers).clone(),
            no_range_header: plain_get,
        };
        let resp = match sh.http.open_range(req).await {
            Ok(r) => r,
            Err(e) => match e.classify() {
                ErrorClass::Temporary if !retry.exhausted(attempt) => {
                    attempt += 1;
                    let wait = retry.delay_for(attempt - 1, e.retry_after);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => continue 'request,
                        _ = cmd.changed() => {
                            sh.table.abandon(work.idx);
                            return Err(Box::new(SegFailure::Stop));
                        }
                    }
                }
                ErrorClass::Restart => {
                    sh.table.abandon(work.idx);
                    return Err(Box::new(SegFailure::Restart {
                        reason: e.message,
                        new_info: None,
                    }));
                }
                _ => {
                    sh.table.abandon(work.idx);
                    return Err(Box::new(SegFailure::Fatal(e)));
                }
            },
        };

        // validate the response against what we asked for. Content-Range must match BOTH
        // ends (audit B3): a server echoing a different end must not have its body trusted —
        // `span` clamps the write, but a too-large `end` would let the server define the
        // segment length instead of the manifest.
        let cr_ok = resp
            .content_range
            .is_some_and(|(s, e, _)| s == start && end.is_none_or(|re| e == re));
        let usable = if plain_get {
            resp.status == 200 || (resp.status == 206 && cr_ok)
        } else {
            resp.status == 206 && cr_ok
        };
        if !usable {
            // Rate limiting / transient server state on a live connection: retry the same
            // segment with backoff instead of tearing the whole download down (audit A6 —
            // three 503 flaps used to consume the whole restart budget).
            if matches!(resp.status, 408 | 429 | 500..=599) {
                if !retry.exhausted(attempt) {
                    attempt += 1;
                    let wait = retry.delay_for(attempt - 1, resp.retry_after);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => continue 'request,
                        _ = cmd.changed() => {
                            sh.table.abandon(work.idx);
                            return Err(Box::new(SegFailure::Stop));
                        }
                    }
                }
                sh.table.abandon(work.idx);
                return Err(Box::new(SegFailure::Fatal(from_status(
                    resp.status,
                    "server kept answering with a transient status",
                ))));
            }
            sh.table.abandon(work.idx);
            let status = resp.status;
            if status == 200 {
                // A 200 can mean "server ignores Range" OR "If-Range failed: the resource
                // changed". At file start with nothing written we may consume a full body —
                // but only when the response validators still match the representation we
                // probed (audit A7: in Single mode there is no other segment to trigger the
                // restart, so accepting a changed body would complete a truncated file).
                let same_resource = plain_get
                    || sh.if_range.is_none()
                    || resp
                        .etag
                        .as_deref()
                        .is_none_or(|e| Some(e) == sh.if_range.as_deref());
                if start == 0 && same_resource {
                    // Nothing written yet at file start: a 200 full body is exactly the bytes
                    // this segment needs — the server simply ignored Range (RFC 9110 §14.2),
                    // or the validators prove the representation is unchanged.
                } else {
                    let new_info = resp.etag.as_ref().map(|etag| ResourceInfo {
                        final_url: sh.url.clone(),
                        etag: Some(etag.clone()),
                        etag_is_weak: false,
                        status: 200,
                        ..Default::default()
                    });
                    let reason = if sh.if_range.is_some() {
                        "server answered 200 to a ranged request (If-Range failed): resource changed".to_string()
                    } else {
                        "server ignored the range request; restarting from scratch".to_string()
                    };
                    return Err(Box::new(SegFailure::Restart { reason, new_info }));
                }
            } else {
                return Err(Box::new(SegFailure::Restart {
                    reason: format!("unexpected range response: HTTP {status}"),
                    new_info: None,
                }));
            }
        }

        // read loop (body taken out of resp exactly once)
        let mut body = resp.body;
        let mut stop_rx = sh.cmd.clone();
        let mut since_ckpt = 0u64;
        loop {
            let chunk = tokio::select! {
                biased;
                _ = stop_rx.changed() => {
                    sh.table.abandon(work.idx);
                    return Err(Box::new(SegFailure::Stop));
                }
                r = body.next_chunk() => r,
            };
            match chunk {
                Ok(Some(bytes)) => {
                    // P0 second line of defense: validate the REAL body, not just the
                    // probe. Only at absolute offset 0 — later chunks and resumed segments
                    // have already passed this check.
                    if !sh.html_allowed
                        && start == 0
                        && sh.table.pos_of(work.idx) == 0
                        && crate::inspect::sniff_kind(&bytes) == crate::inspect::SniffedKind::Html
                    {
                        sh.table.abandon(work.idx);
                        return Err(Box::new(SegFailure::Fatal(EngineError::new(
                            ErrorKind::NotAFile,
                            "the response body is a web page (HTML), not the requested file",
                        ))));
                    }
                    let (off, allowed) = match sh.table.span(work.idx, bytes.len()) {
                        Some(x) => x,
                        None => {
                            // segment boundary reached/passed (steal clamp); stop reading, close conn
                            sh.table.complete(work.idx);
                            return Ok(());
                        }
                    };
                    if let Some(l) = &sh.limiter {
                        // pacing must be cancellable: pause/cancel during a slow drain must
                        // take effect (audit H1 follow-up — acquire used to sit outside the
                        // stop-check)
                        tokio::select! {
                            biased;
                            _ = stop_rx.changed() => {
                                sh.table.abandon(work.idx);
                                return Err(Box::new(SegFailure::Stop));
                            }
                            _ = l.acquire(allowed as u64) => {}
                        }
                    }
                    if let Err(e) = sh.file.write_at(&bytes[..allowed], off) {
                        sh.table.abandon(work.idx);
                        return Err(Box::new(SegFailure::Fatal(from_io(&e))));
                    }
                    sh.table.advance(work.idx, allowed as u64);
                    since_ckpt += allowed as u64;
                    if since_ckpt >= sh.engine.checkpoint_interval {
                        since_ckpt = 0; // monitor persists on its next tick
                                        // audit M5: periodic disk re-check — when the volume is
                                        // critically low, fail honestly NOW (persisted state remains
                                        // resumable once space is freed) instead of hitting ENOSPC
                                        // mid-write and losing the last checkpoint's clarity
                        if engine_storage::low_space(
                            sh.file.path().parent().unwrap_or(std::path::Path::new(".")),
                            engine_storage::SPACE_MARGIN,
                        ) {
                            sh.table.abandon(work.idx);
                            return Err(Box::new(SegFailure::Fatal(EngineError::new(
                                ErrorKind::DiskFull,
                                "destination volume is running out of space —                                  download paused-by-failure; free space and resume",
                            ))));
                        }
                    }
                }
                Ok(None) => {
                    // stream ended
                    let pos = sh.table.pos_of(work.idx);
                    if sh.table.end_of(work.idx).is_none() {
                        // close-delimited: clean EOF is the only end signal (RFC 9112 §6.3)
                        sh.file
                            .sync_data()
                            .map_err(|e| SegFailure::Fatal(from_io(&e)))?;
                        sh.table.complete(work.idx);
                        return Ok(());
                    }
                    if reached_end(sh, work.idx) {
                        sh.file
                            .sync_data()
                            .map_err(|e| SegFailure::Fatal(from_io(&e)))?;
                        sh.table.complete(work.idx);
                        return Ok(());
                    }
                    // premature close: retry from checkpoint
                    attempt += 1;
                    if retry.exhausted(attempt) {
                        sh.table.abandon(work.idx);
                        return Err(Box::new(SegFailure::Fatal(EngineError::new(
                            ErrorKind::Reset,
                            format!("connection closed early at byte {pos} of its range; retries exhausted"),
                        ))));
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(retry.delay_for(attempt - 1, None)) => continue 'request,
                        _ = cmd.changed() => {
                            sh.table.abandon(work.idx);
                            return Err(Box::new(SegFailure::Stop));
                        }
                    }
                }
                Err(e) => match e.classify() {
                    ErrorClass::Temporary if !retry.exhausted(attempt) => {
                        attempt += 1;
                        tokio::time::sleep(retry.delay_for(attempt - 1, e.retry_after)).await;
                        continue 'request;
                    }
                    _ => {
                        sh.table.abandon(work.idx);
                        return Err(Box::new(SegFailure::Fatal(e)));
                    }
                },
            }
        }
    }
}

fn reached_end(sh: &Shared, idx: usize) -> bool {
    match sh.table.end_of(idx) {
        None => true,
        Some(e) => sh.table.abs_pos(idx) > e,
    }
}

async fn stop_workers(tx: &watch::Sender<TaskCommand>, mut joinset: JoinSet<()>) {
    let _ = tx.send(TaskCommand::Pause);
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    while !joinset.is_empty() {
        tokio::select! {
            _ = &mut deadline => { joinset.abort_all(); break; }
            _ = joinset.join_next() => {}
        }
    }
}

/// Handle a worker-reported fatal error. Disk/permission failures become an honest
/// `paused` (partial data kept, resumable after the user fixes the cause), everything
/// else fails the download.
async fn on_failed(
    args: &TaskArgs,
    row: &mut DownloadRow,
    sh: &Arc<Shared>,
    local_tx: &watch::Sender<TaskCommand>,
    joinset: JoinSet<()>,
    e: EngineError,
) -> TaskOutcome {
    stop_workers(local_tx, joinset).await;
    if matches!(e.kind, ErrorKind::DiskFull | ErrorKind::Permission) {
        persist_final_checkpoint(sh, row).await.ok();
        row.state = states::PAUSED.to_string();
        row.error_code = Some(e.kind_code());
        row.error_message = Some(e.message.clone());
        args.db.update_download(row).ok();
        args.events
            .send(Event::Failed {
                id: args.id,
                kind: e.kind_code(),
                message: e.message.clone(),
            })
            .ok();
        return TaskOutcome::Paused;
    }
    finish_failed(args, row, e).await
}

/// Stop workers and either fail (restart limit) or prepare for a fresh attempt.
/// `None` = caller should rebuild and continue the restart loop.
async fn maybe_restart(
    args: &TaskArgs,
    row: &mut DownloadRow,
    local_tx: &watch::Sender<TaskCommand>,
    joinset: JoinSet<()>,
    reason: String,
    restarts: &mut u32,
) -> Option<TaskOutcome> {
    stop_workers(local_tx, joinset).await;
    *restarts += 1;
    if *restarts > 3 {
        let e = EngineError::new(
            ErrorKind::ResourceChanged,
            format!("{reason}; restart limit reached"),
        );
        return Some(finish_failed(args, row, e).await);
    }
    args.events
        .send(Event::Restarting {
            id: args.id,
            reason,
        })
        .ok();
    None
}

/// Final durable checkpoint: fsync data, then persist segment snapshot + aggregate (order is law).
async fn persist_final_checkpoint(
    sh: &Arc<Shared>,
    row: &mut DownloadRow,
) -> Result<(), EngineError> {
    if sh.table.has_dirty() {
        sh.file.sync_data().map_err(|e| from_io(&e))?;
    }
    let snap = sh.table.snapshot();
    let stats = sh.table.stats();
    sh.db
        .persist_snapshot(sh.id, &segs_to_rows(&snap), stats.received as i64)
        .map_err(db_err)?;
    row.received = stats.received as i64;
    Ok(())
}

async fn settle(sh: Arc<Shared>, row: &mut DownloadRow, args: &TaskArgs) -> TaskOutcome {
    // All workers exited: done, or stopped, or a ctrl message raced through.
    let mut cmd = sh.cmd.clone();
    if cmd.borrow_and_update().is_stop() {
        persist_final_checkpoint(&sh, row).await.ok();
        row.state = states::PAUSED.to_string();
        args.db.update_download(row).ok();
        args.events
            .send(Event::DownloadState {
                id: args.id,
                state: states::PAUSED.into(),
            })
            .ok();
        return TaskOutcome::Paused;
    }
    if sh.table.all_done() {
        return finalize(sh, row, args).await;
    }
    // workers exited but work remains with no ctrl message — treat as a pause, safely persisted
    persist_final_checkpoint(&sh, row).await.ok();
    row.state = states::PAUSED.to_string();
    args.db.update_download(row).ok();
    TaskOutcome::Paused
}

/// Completion: truncate (streams) → hash → sync_all → rename → dir fsync → DB commit.
///
/// Finalization state machine — every *required* step's failure fails the task (the engine must
/// never report Completed while finalization failed); only advisory steps (checkpoint, event
/// delivery) log-and-continue. After any failure the part file and DB stay reconcilable: resume
/// either continues from the part file or re-downloads into a collision-safe name. A rename that
/// already happened leaves the DB behind (state failed/downloading) — recovery re-downloads.
async fn finalize(sh: Arc<Shared>, row: &mut DownloadRow, args: &TaskArgs) -> TaskOutcome {
    let stats = sh.table.stats();
    row.received = stats.received as i64;
    // P0 completion proof: when the resource length is known, received MUST equal it.
    // reached_end() enforces this per segment; this is the durable belt-and-braces so a
    // truncated transfer can never be reported Completed.
    if let Some(t) = row.total_size {
        if stats.received != t.unsigned_abs() {
            return finish_failed(
                args,
                row,
                EngineError::new(
                    ErrorKind::InvalidResponse,
                    format!(
                        "truncated transfer: received {} of {} bytes; completion refused",
                        stats.received, t
                    ),
                ),
            )
            .await;
        }
    }
    // advisory: positions are already durable; this last checkpoint only shrinks the resume tail
    if let Err(e) = persist_final_checkpoint(&sh, row).await {
        warn!(id = args.id, error = %e.message, "final checkpoint persist failed (advisory)");
    }

    // streaming (unknown length): the exact size is only known now. This MUST run before the
    // hash — residue beyond `received` (from an earlier longer run) would otherwise be hashed
    // (false CHECKSUM_MISMATCH) and its length would not be durable (audit finding, storage M-a).
    if row.total_size.is_none() {
        let n = stats.received;
        if let Err(e) = sh.file.truncate_to(n) {
            return finish_failed(args, row, from_io(&e)).await;
        }
    } else {
        // residue-tail belt (audit P2): known-length bodies must not ship stale
        // bytes past `total` when preallocation was skipped or had failed
        let t = row.total_size.unwrap() as u64;
        if let Err(e) = sh.file.truncate_to(t) {
            return finish_failed(args, row, from_io(&e)).await;
        }
    }

    // hash before the rename so verification failure keeps the diagnosable part file
    let mut sha256 = None;
    let mut blake3 = None;
    if let Some(vr) = &args.dl.verify {
        let path = sh.file.path().to_path_buf();
        let algo = vr.algo;
        let computed = tokio::task::spawn_blocking(move || compute_hash_blocking(&path, algo))
            .await
            .map_err(|e| EngineError::new(ErrorKind::Io, format!("hash task failed: {e}")))
            .and_then(|r| r.map_err(|e| from_io(&e)));
        match computed {
            Ok(h) if h.eq_ignore_ascii_case(&vr.expected) => {
                match vr.algo {
                    HashAlgo::Sha256 => sha256 = Some(h),
                    HashAlgo::Blake3 => blake3 = Some(h),
                }
                row.verified = true;
            }
            Ok(h) => {
                let e = EngineError::new(
                    ErrorKind::InvalidResponse,
                    format!(
                        "checksum mismatch: expected {}={}, computed {h}",
                        vr.algo.name(),
                        vr.expected
                    ),
                );
                if let Err(db_e) = args.db.record_error(args.id, &err_row(&e, None)) {
                    warn!(id = args.id, error = %db_e, "could not record checksum-mismatch error row");
                }
                row.state = states::FAILED.to_string();
                row.error_code = Some("CHECKSUM_MISMATCH".into());
                row.error_message = Some(e.message.clone());
                if let Err(db_e) = args.db.update_download(row) {
                    warn!(id = args.id, error = %db_e, "could not persist CHECKSUM_MISMATCH state");
                }
                args.events
                    .send(Event::Failed {
                        id: args.id,
                        kind: "CHECKSUM_MISMATCH".into(),
                        message: e.message.clone(),
                    })
                    .ok();
                return TaskOutcome::Failed(e);
            }
            Err(e) => return finish_failed(args, row, e).await,
        }
    }

    if let Err(e) = sh.file.sync_all() {
        return finish_failed(args, row, from_io(&e)).await;
    }

    let desired = args.dest_dir.join(&row.final_name);
    let final_path = if args.dl.overwrite {
        // std::fs::rename replaces an existing target on BOTH platforms (POSIX
        // rename(2); MoveFileExW with MOVEFILE_REPLACE_EXISTING on Windows) — a
        // pre-emptive remove is wrong everywhere: it would destroy the user's file
        // if the rename then failed (review finding; the old cfg(windows) branch
        // removed the target before renaming and could lose it on rename failure)
        match std::fs::rename(sh.file.path(), &desired) {
            Ok(_) => {
                if let Err(e) = engine_storage::sync_dir(&desired) {
                    // the rename happened but may not survive power loss: completion would
                    // be un-durable → fail (resume re-downloads into a collision-safe name)
                    return finish_failed(args, row, from_io(&e)).await;
                }
                desired
            }
            Err(e) => return finish_failed(args, row, from_io(&e)).await,
        }
    } else {
        match engine_storage::finalize(sh.file.path(), &desired) {
            Ok(p) => p,
            Err(e) => return finish_failed(args, row, from_io(&e)).await,
        }
    };
    // collision-safe renames change the actual name: the row must point at the real file,
    // or status/verify would hash the pre-existing neighbor instead (audit finding M3)
    if let Some(new_name) = final_path.file_name() {
        row.final_name = new_name.to_string_lossy().into_owned();
    }

    row.state = states::COMPLETED.to_string();
    row.temp_path = None;
    row.completed_at = Some(engine_state::now_iso());
    row.sha256 = sha256.clone();
    row.blake3 = blake3.clone();
    if let Err(e) = args.db.update_download(row) {
        // the file is renamed but completion cannot be persisted: reporting Completed here
        // would leave a row that claims downloading with no part file → never do that
        let db_err = EngineError::new(ErrorKind::Io, format!("final state commit failed: {e}"));
        let failed_row = row_clone_as_failed(row, &db_err);
        let _ = args.db.update_download(&failed_row);
        args.events
            .send(Event::Failed {
                id: args.id,
                kind: db_err.kind_code(),
                message: db_err.message.clone(),
            })
            .ok();
        return TaskOutcome::Failed(db_err);
    }
    args.events
        .send(Event::Completed {
            id: args.id,
            path: final_path.display().to_string(),
            size: stats.received,
            sha256: sha256.clone(),
            blake3: blake3.clone(),
        })
        .ok();
    info!(id = args.id, path = %final_path.display(), size = stats.received, "download completed");
    TaskOutcome::Completed {
        path: final_path,
        size: stats.received,
        sha256,
        blake3,
    }
}

/// A copy of `row` with failure state applied (used when the row must not be mutated further).
fn row_clone_as_failed(row: &DownloadRow, e: &EngineError) -> DownloadRow {
    let mut r = row.clone();
    r.state = states::FAILED.to_string();
    r.error_code = Some(e.kind_code());
    r.error_message = Some(e.message.clone());
    r
}

async fn finish_failed(args: &TaskArgs, row: &mut DownloadRow, e: EngineError) -> TaskOutcome {
    row.state = states::FAILED.to_string();
    row.error_code = Some(e.kind_code());
    row.error_message = Some(e.message.clone());
    if let Err(db_e) = args.db.update_download(row) {
        warn!(id = args.id, error = %db_e, "FAILED state could not be persisted (row stays stale; resume re-downloads safely)");
    }
    if let Err(db_e) = args.db.record_error(args.id, &err_row(&e, None)) {
        warn!(id = args.id, error = %db_e, "error row could not be recorded");
    }
    args.events
        .send(Event::Failed {
            id: args.id,
            kind: e.kind_code(),
            message: e.message.clone(),
        })
        .ok();
    warn!(id = args.id, code = e.kind_code(), msg = %e.message, "download failed");
    TaskOutcome::Failed(e)
}

fn err_row(e: &EngineError, seg: Option<i64>) -> ErrorRow {
    ErrorRow {
        code: e.kind_code(),
        message: e.message.clone(),
        class: format!("{:?}", e.classify()).to_lowercase(),
        segment_idx: e.segment_idx.or(seg),
    }
}

fn db_err(e: engine_state::StateError) -> EngineError {
    EngineError::new(ErrorKind::Io, format!("state: {e}"))
}
