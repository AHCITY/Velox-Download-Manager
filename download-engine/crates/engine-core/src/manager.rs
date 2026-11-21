//! DownloadManager: the engine facade every client (CLI, API, future UIs) talks to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{watch, Semaphore};
use tracing::info;

use engine_state::{states, Db, DownloadRow};

use crate::config::{DownloadConfig, EngineConfig, HashAlgo};
use crate::error::{from_io, from_status, EngineError, ErrorKind};
use crate::events::{make_bus, Event, EventBus};
use crate::inspect::RangeSupport;
use crate::naming;
use crate::ports::{HttpPort, RequestHeaders};
use crate::segments::SegmentTable;
use crate::task::{compute_hash_blocking, run_task, TaskArgs, TaskCommand};

#[derive(Debug, Serialize)]
pub struct AddedDownload {
    pub id: i64,
    pub filename: String,
    pub dest_dir: String,
    pub temp_path: String,
    pub total_size: Option<u64>,
    pub accept_ranges: String,
    pub resumable: &'static str,
    pub resumed_from: u64,
    pub existing_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentView {
    pub idx: i64,
    pub start: u64,
    pub end: Option<u64>,
    pub pos: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub id: i64,
    pub url: String,
    pub final_name: String,
    pub dest_dir: String,
    pub state: String,
    pub mode: String,
    pub total_size: Option<u64>,
    pub received: u64,
    pub accept_ranges: String,
    pub resumable: &'static str,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub running: bool,
    pub connections_active: usize,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub sha256: Option<String>,
    pub blake3: Option<String>,
    pub verified: bool,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub segments: Vec<SegmentView>,
    pub recent_errors: Vec<String>,
    /// Audit M5 (2026-11-21): a COMPLETED row whose final file no longer exists on
    /// disk (deleted/moved externally). The row stays honest: state is not rewritten;
    /// the UI can surface "file missing" instead of a phantom entry.
    pub file_missing: bool,
}

#[derive(Debug, Serialize)]
pub struct VerifyOutcome {
    pub id: i64,
    pub algo: String,
    pub computed: String,
    pub expected: Option<String>,
    pub matches: Option<bool>,
}

#[derive(Clone)]
struct RunningTask {
    cmd: watch::Sender<TaskCommand>,
    live_table: Arc<Mutex<Option<Arc<SegmentTable>>>>,
}

pub struct DownloadManager {
    engine: EngineConfig,
    http: Arc<dyn HttpPort>,
    db: Arc<Db>,
    events: EventBus,
    host_sems: Mutex<HashMap<String, Arc<Semaphore>>>,
    running: Mutex<HashMap<i64, RunningTask>>,
    /// Admission control: a permit per running download (None = unlimited).
    download_slots: Option<Arc<Semaphore>>,
}

impl DownloadManager {
    pub fn new(engine: EngineConfig, http: Arc<dyn HttpPort>) -> Result<Arc<Self>, EngineError> {
        let db = Db::open(&engine.data_dir.join("state.db"))
            .map_err(|e| EngineError::new(ErrorKind::Io, format!("cannot open state db: {e}")))?;
        // audit D8 housekeeping: reclaim lock files abandoned >7 days ago (race-free —
        // see gc_stale_locks; recent stale files are claimed by acquire() anyway)
        engine_storage::gc_stale_locks(
            &engine.data_dir.join("locks"),
            std::time::Duration::from_secs(7 * 24 * 3600),
        );
        let (events, _) = make_bus(engine.event_capacity);
        let download_slots = engine
            .max_concurrent_downloads
            .map(|n| Arc::new(Semaphore::new(n.max(1))));
        Ok(Arc::new(Self {
            engine,
            http,
            db: Arc::new(db),
            events,
            host_sems: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            download_slots,
        }))
    }

    pub fn events(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn engine_config(&self) -> &EngineConfig {
        &self.engine
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.engine.data_dir
    }

    /// Raw DB access for the API layer (error history etc.).
    /// The HTTP backend as a trait object (for backend-specific extensions).
    pub fn http(&self) -> &Arc<dyn crate::ports::HttpPort> {
        &self.http
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    fn host_semaphore(&self, url: &str) -> Arc<Semaphore> {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| "unknown".into());
        let mut map = self.host_sems.lock().unwrap();
        map.entry(host)
            .or_insert_with(|| {
                Arc::new(Semaphore::new(self.engine.max_connections_per_host.max(1)))
            })
            .clone()
    }

    fn running_snapshot(&self, id: i64) -> Option<RunningTask> {
        self.running.lock().unwrap().get(&id).cloned()
    }

    /// Inspection with transient-error retry (PROTOCOL_COMPATIBILITY §1.6): 429/5xx/timeouts
    /// are retried per the engine policy, honoring Retry-After when the server provided one.
    /// Uses `HttpPort::resolve`, so backend URL semantics (Drive confirm pages, media
    /// extraction) apply before anything is registered on disk.
    /// Proactive destination check (audit M5): create the directory if needed, then
    /// require free space ≥ content_length + SPACE_MARGIN when the size is known.
    fn preflight_checked(
        dest_dir: &std::path::Path,
        total: Option<u64>,
    ) -> Result<(), EngineError> {
        std::fs::create_dir_all(dest_dir).map_err(|e| {
            EngineError::new(
                ErrorKind::Io,
                format!(
                    "destination directory is unavailable: {} ({e})",
                    dest_dir.display()
                ),
            )
        })?;
        if let Some(total) = total {
            let free = engine_storage::available_space(dest_dir).map_err(|e| {
                EngineError::new(
                    ErrorKind::Io,
                    format!("cannot query free space on {}: {e}", dest_dir.display()),
                )
            })?;
            if free < total.saturating_add(engine_storage::SPACE_MARGIN) {
                return Err(EngineError::new(
                    ErrorKind::DiskFull,
                    format!(
                        "destination has {free} bytes free but this download needs                          {total} bytes (plus a {} MiB safety margin) — free up space                          or choose another folder",
                        engine_storage::SPACE_MARGIN / (1024 * 1024)
                    ),
                ));
            }
        }
        Ok(())
    }

    async fn resolve_with_retry(
        &self,
        url: &str,
        headers: &RequestHeaders,
    ) -> Result<crate::inspect::ResourceInfo, EngineError> {
        // audit D4: resolve (Drive confirm walk, yt-dlp extraction) can take seconds to
        // minutes before any row exists — announce it so UIs are not frozen spinners
        self.events
            .send(crate::events::Event::Resolving {
                url: url.to_string(),
            })
            .ok();
        let mut attempts = 0u32;
        loop {
            match self.http.resolve(url, headers).await {
                Ok(i) if crate::inspect::probe_usable(&i) => {
                    self.events
                        .send(crate::events::Event::Resolved {
                            url: url.to_string(),
                            note: i.resolve_note.clone(),
                        })
                        .ok();
                    return Ok(i);
                }
                Ok(i) => {
                    let mut e = from_status(i.status, "inspection failed");
                    if let Some(ra) = i.retry_after {
                        e = e.with_retry_after(ra);
                    }
                    if e.classify() == crate::error::ErrorClass::Temporary
                        && !self.engine.retry.exhausted(attempts)
                    {
                        attempts += 1;
                        tokio::time::sleep(
                            self.engine.retry.delay_for(attempts - 1, e.retry_after),
                        )
                        .await;
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => {
                    if e.classify() == crate::error::ErrorClass::Temporary
                        && !self.engine.retry.exhausted(attempts)
                    {
                        attempts += 1;
                        tokio::time::sleep(
                            self.engine.retry.delay_for(attempts - 1, e.retry_after),
                        )
                        .await;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }

    /// URL gate shared by add/inspect (audit A12): http/https only, authority required,
    /// embedded userinfo rejected — credentials belong in AuthConfig (URL-borne credentials
    /// are persisted in the manifest and echoed in status output).
    ///
    /// Delegates to the `url_adapter` seam (architecture review §3); the gate itself
    /// lives there now, tested in isolation.
    fn validate_url(url: &str) -> Result<url::Url, EngineError> {
        crate::url_adapter::validate(url)
    }

    /// Inspect a URL without registering a download (UI preview; UI_READINESS §2.2).
    /// Same validation and probe-retry semantics as `add`.
    pub async fn inspect(
        &self,
        url: &str,
        cfg: DownloadConfig,
    ) -> Result<crate::inspect::ResourceInfo, EngineError> {
        Self::validate_url(url)?;
        let headers = RequestHeaders::from_config(
            &cfg.headers,
            &cfg.referer,
            &cfg.cookies,
            &cfg.auth,
            cfg.insecure_tls,
        );
        self.resolve_with_retry(url, &headers).await
    }

    /// Inspect a URL and register a download without starting it.
    pub async fn add(&self, url: &str, cfg: DownloadConfig) -> Result<AddedDownload, EngineError> {
        Self::validate_url(url)?;
        let headers = RequestHeaders::from_config(
            &cfg.headers,
            &cfg.referer,
            &cfg.cookies,
            &cfg.auth,
            cfg.insecure_tls,
        );
        let info = self.resolve_with_retry(url, &headers).await?;
        if !crate::inspect::probe_usable(&info) {
            return Err(from_status(info.status, "inspection failed"));
        }
        // P0 gate at registration time: a web page (HTML) or compressed transfer never
        // becomes a download row at all — the user sees the real reason immediately.
        crate::task::page_gate(&info, &cfg)?;

        let dest_dir = cfg
            .out_dir
            .clone()
            .unwrap_or_else(|| self.engine.default_out_dir.clone());
        let final_name = {
            let n = naming::choose_filename(&info, cfg.filename.as_deref(), 0);
            if n == "download-0" {
                "download".to_string()
            } else {
                n
            }
        };
        let dest_str = dest_dir.display().to_string();

        // audit M5: proactive disk-full handling — refuse to START when the destination
        // volume cannot hold the file, and distinguish an unavailable destination from
        // low disk space (both honest failures, different remedies)
        Self::preflight_checked(&dest_dir, info.content_length)?;

        // duplicate detection: same URL + destination + name → point at the existing record
        if let Ok(Some(existing)) = self.db.find_same_destination(url, &dest_str, &final_name) {
            return Ok(AddedDownload {
                id: existing.id,
                filename: existing.final_name.clone(),
                dest_dir: existing.dest_dir.clone(),
                temp_path: existing.temp_path.clone().unwrap_or_default(),
                total_size: existing.total_size.map(|t| t as u64),
                accept_ranges: existing.accept_ranges.clone(),
                resumable: resumability(&existing),
                resumed_from: existing.received.max(0) as u64,
                existing_id: Some(existing.id),
            });
        }

        let resumable = match info.range_support {
            RangeSupport::Yes => "yes",
            RangeSupport::No => {
                if info.content_length.is_some() {
                    "conditional"
                } else {
                    "no"
                }
            }
            RangeSupport::Unknown => "unknown",
        };

        let row = DownloadRow {
            url: url.to_string(),
            resolved_url: Some(info.final_url.clone()),
            dest_dir: dest_str.clone(),
            final_name: final_name.clone(),
            total_size: info.content_length.map(|t| t as i64),
            etag: info.etag.clone(),
            etag_is_weak: info.etag_is_weak,
            last_modified: info.last_modified.clone(),
            mime: info.mime.clone(),
            content_encoding: info.content_encoding.clone(),
            accept_ranges: match info.range_support {
                RangeSupport::Yes => "yes",
                RangeSupport::No => "no",
                RangeSupport::Unknown => "unknown",
            }
            .to_string(),
            mode: "unknown".into(),
            state: states::CREATED.to_string(),
            max_connections: crate::plan::effective_max_connections(&self.engine, &cfg) as i64,
            config_json: serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into()),
            ..Default::default()
        };
        let id = self.db.insert_download(&row).map_err(db_err)?;
        let temp_path = dest_dir.join(format!("{}.part-{:x}", final_name, id));
        let mut row = row;
        row.id = id;
        row.temp_path = Some(temp_path.display().to_string());
        self.db.update_download(&row).map_err(db_err)?;
        info!(id, url, name = %final_name, size = ?info.content_length, "download added");

        Ok(AddedDownload {
            id,
            filename: final_name,
            dest_dir: dest_str,
            temp_path: temp_path.display().to_string(),
            total_size: info.content_length,
            accept_ranges: row.accept_ranges.clone(),
            resumable,
            resumed_from: 0,
            existing_id: None,
        })
    }

    /// Start (or resume) a registered download.
    pub async fn start(self: Arc<Self>, id: i64) -> Result<(), EngineError> {
        if self.running.lock().unwrap().contains_key(&id) {
            return Ok(()); // already running
        }
        let mut row = self
            .db
            .get_download(id)
            .map_err(db_err)?
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidConfig, "unknown download id"))?;
        if row.state == states::COMPLETED {
            return Err(EngineError::new(
                ErrorKind::InvalidConfig,
                "download already completed",
            ));
        }
        if row.state == states::CANCELLED {
            return Err(EngineError::new(
                ErrorKind::InvalidConfig,
                "download was cancelled",
            ));
        }
        // admission control (UI_READINESS §2.3): when the cap is reached, start()
        // returns IMMEDIATELY with the row marked queued and the runner waits for the
        // slot in the background. Waiting inline deadlocked the sequential v1 stdio
        // loop: with all slots busy, `download.start` parked inside handle_line and
        // even `download.pause` for OTHER downloads became unreachable.
        let slot = match &self.download_slots {
            Some(slots) => match slots.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => {
                    let mut q = row.clone();
                    q.state = states::QUEUED.to_string();
                    self.db.update_download(&q).map_err(db_err)?;
                    self.events
                        .send(crate::events::Event::DownloadState {
                            id,
                            state: states::QUEUED.to_string(),
                        })
                        .ok();
                    None // permit acquired inside the runner after the wait
                }
            },
            None => None,
        };
        let cfg: DownloadConfig = serde_json::from_str(&row.config_json).unwrap_or_default();
        let temp = match row.temp_path.as_deref().filter(|s| !s.is_empty()) {
            Some(t) => PathBuf::from(t),
            None => {
                // self-heal a row whose insert/update was split by a crash (audit B5): assign
                // the canonical temp path now, before the lock/PartFile machinery needs it
                let t = PathBuf::from(&row.dest_dir)
                    .join(format!("{}.part-{:x}", row.final_name, row.id));
                row.temp_path = Some(t.display().to_string());
                let _ = self.db.update_download(&row);
                t
            }
        };

        // rename-orphan adoption (RECOVERY_DESIGN §6): the crash window "after rename, before
        // DB commit" leaves a final file with the row still in-flight. Adopt ONLY with
        // full-content proof — an expected checksum in the download config, matching size,
        // and a recomputed hash. Filename or size alone is never evidence; without a
        // configured checksum we keep the safe re-download path.
        if let (Some(vr), Some(total)) = (&cfg.verify, row.total_size) {
            if !temp.exists() {
                let candidate = PathBuf::from(&row.dest_dir).join(&row.final_name);
                if let Ok(meta) = std::fs::metadata(&candidate) {
                    if meta.len() == total as u64 {
                        let verify_path = candidate.clone();
                        let algo = vr.algo;
                        let expected = vr.expected.clone();
                        let hash = tokio::task::spawn_blocking(move || {
                            crate::task::compute_hash_blocking(&verify_path, algo)
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok());
                        let verified = hash
                            .as_ref()
                            .is_some_and(|h| h.eq_ignore_ascii_case(&expected));
                        if verified {
                            info!(id, path = %candidate.display(), "adopting rename-orphan (hash-verified)");
                            row.state = states::COMPLETED.to_string();
                            row.temp_path = None;
                            row.completed_at = Some(engine_state::now_iso());
                            if vr.algo == crate::config::HashAlgo::Sha256 {
                                row.sha256 = hash.clone();
                            } else {
                                row.blake3 = hash.clone();
                            }
                            row.verified = true;
                            self.db.update_download(&row).map_err(db_err)?;
                            self.events
                                .send(crate::events::Event::Completed {
                                    id,
                                    path: candidate.display().to_string(),
                                    size: total as u64,
                                    sha256: row.sha256.clone(),
                                    blake3: row.blake3.clone(),
                                })
                                .ok();
                            return Ok(());
                        }
                    }
                }
            }
        }

        // cross-process lock: two engine processes must never write the same part file.
        // audit D8: lock files live in the engine data dir (internal), NOT beside the
        // part file — the user's destination directory stays clean.
        let lock =
            engine_storage::DownloadLock::acquire(&temp, &self.engine.data_dir.join("locks"))
                .map_err(|e| {
                    EngineError::new(
                        ErrorKind::FileLock,
                        format!("cannot lock {}: {e}", temp.display()),
                    )
                })?;
        let file = engine_storage::PartFile::create(temp).map_err(|e| from_io(&e))?;

        let args = TaskArgs {
            id,
            engine: self.engine.clone(),
            dl: cfg,
            http: self.http.clone(),
            db: self.db.clone(),
            events: self.events.clone(),
            host_permits: self
                .host_semaphore(&row.resolved_url.clone().unwrap_or_else(|| row.url.clone())),
            url: row.resolved_url.clone().unwrap_or_else(|| row.url.clone()),
            dest_dir: PathBuf::from(&row.dest_dir),
            file: Arc::new(file),
            live_table: Arc::new(Mutex::new(None)),
        };
        let (cmd_tx, cmd_rx) = watch::channel(TaskCommand::Run);
        self.running.lock().unwrap().insert(
            id,
            RunningTask {
                cmd: cmd_tx.clone(),
                live_table: args.live_table.clone(),
            },
        );

        // self-cleaning runner: removes the running entry when the task settles;
        // the download lock is held for the task's whole lifetime (Drop releases it)
        let this = self.clone();
        let slots = self.download_slots.clone();
        tokio::spawn(async move {
            let _lock = lock;
            // admission: use the immediately-acquired permit, or wait here — NOT on the
            // API thread. After the wait, re-check the row: it may have been paused,
            // cancelled or removed while queued (a pause recorded while queued is
            // honored — the download must not start against explicit user intent).
            let _slot = match (slot, slots) {
                (Some(p), _) => Some(p),
                (None, Some(sem)) => {
                    let permit = match sem.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => {
                            this.running.lock().unwrap().remove(&id);
                            return;
                        }
                    };
                    let proceed = match this.db.get_download(id) {
                        Ok(Some(r)) => {
                            matches!(r.state.as_str(), states::CREATED | states::QUEUED)
                        }
                        _ => false,
                    };
                    if !proceed {
                        this.running.lock().unwrap().remove(&id);
                        return;
                    }
                    Some(permit)
                }
                (None, None) => None,
            };
            // panic-safe cleanup: run_task executes in an inner task so a panic cannot
            // skip the running-entry removal (which used to zombie the download —
            // pause/cancel then hit their settle timeouts forever). The row is marked
            // failed so the user sees what happened instead of a stuck spinner.
            let inner = tokio::spawn(run_task(args, cmd_rx));
            let outcome = match inner.await {
                Ok(o) => Some(o),
                Err(_) => {
                    if let Ok(Some(mut r)) = this.db.get_download(id) {
                        r.state = states::FAILED.to_string();
                        r.error_message =
                            Some("internal task panic (worker crashed); safe to retry".into());
                        let _ = this.db.update_download(&r);
                        this.events
                            .send(crate::events::Event::Failed {
                                id,
                                kind: "PANIC".to_string(),
                                message: "task panicked; state preserved for retry".to_string(),
                            })
                            .ok();
                    }
                    None
                }
            };
            this.running.lock().unwrap().remove(&id);
            // drop host semaphores no running task references anymore (audit A8: unbounded map)
            this.host_sems
                .lock()
                .unwrap()
                .retain(|_, v| Arc::strong_count(v) > 1);
            let _ = outcome;
        });
        Ok(())
    }

    pub async fn pause(&self, id: i64) -> Result<(), EngineError> {
        // queued rows are paused by state alone: the runner re-checks after admission
        // and will not start a PAUSED row
        if let Some(mut row) = self.db.get_download(id).map_err(db_err)? {
            if row.state == states::QUEUED {
                row.state = states::PAUSED.to_string();
                self.db.update_download(&row).map_err(db_err)?;
                self.running.lock().unwrap().remove(&id);
                self.events
                    .send(crate::events::Event::DownloadState {
                        id,
                        state: states::PAUSED.to_string(),
                    })
                    .ok();
                return Ok(());
            }
        }
        if let Some(entry) = self.running_snapshot(id) {
            let _ = entry.cmd.send(TaskCommand::Pause);
            // honesty fix: report when the task did NOT settle — "ok" used to mask
            // a download that kept running (e.g. parked on a host permit) and could
            // still complete
            if !wait_until_settled(self, id, Duration::from_secs(30)).await {
                return Err(EngineError::new(
                    ErrorKind::Io,
                    "pause requested; task still settling (it may be finalizing) — retry",
                ));
            }
            return Ok(());
        }
        // not running: mark paused directly (e.g. after an engine restart)
        if let Some(mut row) = self.db.get_download(id).map_err(db_err)? {
            if row.state == states::DOWNLOADING || row.state == states::CREATED {
                row.state = states::PAUSED.to_string();
                self.db.update_download(&row).map_err(db_err)?;
            }
        }
        Ok(())
    }

    /// Cancel and forget: removes the manifest row and the part file.
    pub async fn cancel(&self, id: i64) -> Result<(), EngineError> {
        if let Some(entry) = self.running_snapshot(id) {
            let _ = entry.cmd.send(TaskCommand::Cancel);
            if !wait_until_settled(self, id, Duration::from_secs(60)).await {
                // never delete state under a live task: finalize may be mid-rename and the
                // workers would keep writing to an unlinked inode (audit B1)
                return Err(EngineError::new(
                    ErrorKind::Io,
                    "cancel timed out; task still settling",
                ));
            }
            // re-read: a cancel racing natural completion must not orphan the final file
            // (the row says completed and temp_path is already None)
            let state = self
                .db
                .get_download(id)
                .ok()
                .flatten()
                .map(|r| r.state)
                .unwrap_or_default();
            if state == states::COMPLETED {
                return Ok(());
            }
        }
        self.remove_partial(id)
    }

    /// Remove the record (and part file, if any) entirely.
    /// A COMPLETED download is protected: its file has been finalized to the user's
    /// destination, so removing the row would orphan the file (no record, file still
    /// on disk). Callers must use an explicit "remove completed" semantic instead.
    pub fn remove_partial(&self, id: i64) -> Result<(), EngineError> {
        let row = self.db.get_download(id).map_err(db_err)?;
        if let Some(r) = &row {
            if r.state == states::COMPLETED {
                return Err(EngineError::new(
                    ErrorKind::InvalidConfig,
                    "download is completed; its file stays — remove the row explicitly if intended",
                ));
            }
        }
        self.db.delete_download(id).map_err(db_err)?;
        if let Some(r) = row {
            if let Some(t) = r.temp_path {
                std::fs::remove_file(&t).ok(); // may already be renamed/absent
            }
        }
        Ok(())
    }

    pub fn status(&self, id: i64) -> Result<StatusView, EngineError> {
        let row = self
            .db
            .get_download(id)
            .map_err(db_err)?
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidConfig, "unknown download id"))?;
        let mut view = self.row_to_view(&row);
        if let Some(entry) = self.running_snapshot(id) {
            if let Some(table) = entry.live_table.lock().unwrap().clone() {
                let stats = table.stats();
                view.received = stats.received;
                view.running = true;
                view.connections_active = stats.active;
                view.segments = table
                    .snapshot()
                    .into_iter()
                    .map(|s| SegmentView {
                        idx: s.idx,
                        start: s.start,
                        end: s.end,
                        pos: s.pos,
                        completed: s.completed,
                    })
                    .collect();
            }
        }
        if view.segments.is_empty() {
            view.segments = self
                .db
                .segments(id)
                .map_err(db_err)?
                .into_iter()
                .map(|s| SegmentView {
                    idx: s.idx,
                    start: s.start_off.max(0) as u64,
                    end: s.end_off.map(|e| e as u64),
                    pos: s.pos.max(0) as u64,
                    completed: s.completed,
                })
                .collect();
        }
        Ok(view)
    }

    pub fn list(&self) -> Result<Vec<StatusView>, EngineError> {
        Ok(self
            .db
            .list_downloads()
            .map_err(db_err)?
            .iter()
            .map(|r| self.row_to_view(r))
            .collect())
    }

    fn row_to_view(&self, row: &DownloadRow) -> StatusView {
        StatusView {
            id: row.id,
            url: row.url.clone(),
            final_name: row.final_name.clone(),
            dest_dir: row.dest_dir.clone(),
            state: row.state.clone(),
            mode: row.mode.clone(),
            total_size: row.total_size.map(|t| t as u64),
            received: row.received.max(0) as u64,
            accept_ranges: row.accept_ranges.clone(),
            resumable: resumability(row),
            etag: row.etag.clone(),
            last_modified: row.last_modified.clone(),
            running: false,
            connections_active: 0,
            error_code: row.error_code.clone(),
            error_message: row.error_message.clone(),
            sha256: row.sha256.clone(),
            blake3: row.blake3.clone(),
            verified: row.verified,
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
            completed_at: row.completed_at.clone(),
            segments: Vec::new(),
            recent_errors: Vec::new(),
            file_missing: row.state == states::COMPLETED
                && !std::path::Path::new(&row.dest_dir)
                    .join(&row.final_name)
                    .exists(),
        }
    }

    /// Recompute a checksum for a completed download; stores it on the record.
    pub async fn verify(&self, id: i64) -> Result<VerifyOutcome, EngineError> {
        let row = self
            .db
            .get_download(id)
            .map_err(db_err)?
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidConfig, "unknown download id"))?;
        let path = match row.state.as_str() {
            states::COMPLETED => PathBuf::from(&row.dest_dir).join(&row.final_name),
            _ => {
                return Err(EngineError::new(
                    ErrorKind::InvalidConfig,
                    "download is not completed; verify needs the final file",
                ))
            }
        };
        if !path.exists() {
            return Err(EngineError::new(
                ErrorKind::Io,
                format!(
                    "final file is missing from disk ({}): it was deleted or moved                      externally; verify is impossible for a missing file",
                    path.display()
                ),
            ));
        }
        let (algo, expected) = if row.sha256.is_some() {
            (HashAlgo::Sha256, row.sha256.clone())
        } else if row.blake3.is_some() {
            (HashAlgo::Blake3, row.blake3.clone())
        } else if let Ok(cfg) = serde_json::from_str::<DownloadConfig>(&row.config_json) {
            match cfg.verify {
                Some(v) => (v.algo, Some(v.expected)),
                None => (HashAlgo::Sha256, None),
            }
        } else {
            (HashAlgo::Sha256, None)
        };
        let computed = tokio::task::spawn_blocking(move || compute_hash_blocking(&path, algo))
            .await
            .map_err(|e| EngineError::new(ErrorKind::Io, format!("verify task failed: {e}")))?
            .map_err(|e| from_io(&e))?;
        let matches = expected
            .as_ref()
            .map(|exp| computed.eq_ignore_ascii_case(exp));
        let mut updated = row.clone();
        match algo {
            HashAlgo::Sha256 => updated.sha256 = Some(computed.clone()),
            HashAlgo::Blake3 => updated.blake3 = Some(computed.clone()),
        }
        updated.verified = matches.unwrap_or(false);
        self.db.update_download(&updated).map_err(db_err)?;
        self.events
            .send(Event::Verified {
                id,
                ok: matches.unwrap_or(true),
                algo: algo.name().into(),
            })
            .ok();
        Ok(VerifyOutcome {
            id,
            algo: algo.name().into(),
            computed,
            expected,
            matches,
        })
    }
}

/// Wait until a task leaves the running map. Returns false on timeout (callers must then
/// refuse destructive operations).
async fn wait_until_settled(mgr: &DownloadManager, id: i64, max_wait: Duration) -> bool {
    let deadline = tokio::time::sleep(max_wait);
    tokio::pin!(deadline);
    loop {
        if mgr.running.lock().unwrap().get(&id).is_none() {
            return true;
        }
        tokio::select! {
            _ = &mut deadline => return false,
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

fn resumability(row: &DownloadRow) -> &'static str {
    match row.accept_ranges.as_str() {
        "yes" => "yes",
        "no" => {
            if row.total_size.is_some() {
                "conditional"
            } else {
                "no"
            }
        }
        _ => "unknown",
    }
}

fn db_err(e: engine_state::StateError) -> EngineError {
    EngineError::new(ErrorKind::Io, format!("state: {e}"))
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    #[test]
    fn preflight_refuses_sizes_no_volume_can_hold() {
        let dir = tempfile::tempdir().unwrap();
        // 1 PiB + margin against any temp volume → DiskFull with an honest message
        let e = DownloadManager::preflight_checked(dir.path(), Some(1 << 50)).unwrap_err();
        assert!(matches!(e.kind, ErrorKind::DiskFull), "got {:?}", e.kind);
        assert!(e.message.contains("bytes free"), "message: {}", e.message);
        // a realistic size passes
        assert!(DownloadManager::preflight_checked(dir.path(), Some(64 * 1024)).is_ok());
        // unknown size: only availability is checked
        assert!(DownloadManager::preflight_checked(dir.path(), None).is_ok());
    }

    #[test]
    fn preflight_distinguishes_unavailable_destination() {
        let e = DownloadManager::preflight_checked(
            std::path::Path::new("/proc/velox-cannot-create-this/dest"),
            Some(1024),
        )
        .unwrap_err();
        assert!(
            e.message.contains("unavailable"),
            "must say 'unavailable', got: {}",
            e.message
        );
        assert!(!matches!(e.kind, ErrorKind::DiskFull));
    }
}
