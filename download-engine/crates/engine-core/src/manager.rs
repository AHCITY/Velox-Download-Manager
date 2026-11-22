//! DownloadManager: the engine facade every client (CLI, API, future UIs) talks to.

use std::collections::{HashMap, HashSet};
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
    /// Per-download speed cap from the stored DownloadConfig (UI: Network screen).
    pub bandwidth_limit_bps: Option<u64>,
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
    /// MediaPipeline merge stage (review §9), implemented in engine-media and
    /// injected from outside core — the transfer engine never links ffmpeg.
    media_merge: Mutex<Option<Arc<dyn crate::ports::MediaMergePort>>>,
    /// Kill triggers for in-flight merge phases (id → sender); cancel() uses them.
    merge_triggers: Mutex<HashMap<i64, tokio::sync::watch::Sender<bool>>>,
    /// Single-flight guard for merge phases (video ids currently merging): two
    /// concurrent merges into the same row would both write the same output.
    merging: Mutex<HashSet<i64>>,
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
        // merge-crash recovery (audit P0-2): a row left `merging` by a process death
        // is stuck forever — no trigger exists after restart and the half-written
        // ffmpeg output would leak into the out dir. Sweep at startup instead.
        Self::sweep_stale_merging(&db);
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
            media_merge: Mutex::new(None),
            merge_triggers: Mutex::new(HashMap::new()),
            merging: Mutex::new(HashSet::new()),
        }))
    }

    /// Inject the MediaPipeline merge stage (engine-media). None disables merges:
    /// `merge_streams` then fails honestly instead of pretending.
    pub fn set_media_merge(&self, port: Option<Arc<dyn crate::ports::MediaMergePort>>) {
        *self.media_merge.lock().unwrap() = port;
    }

    /// Startup sweep for rows left in MERGING by a process death (audit P0-2).
    /// The half-written merge output is deleted (it lives at `<final>.merging.part`
    /// since the temp-file lifecycle change) and the row is failed honestly with
    /// MERGE_INTERRUPTED so the user re-runs the merge instead of owning garbage.
    fn sweep_stale_merging(db: &Db) {
        let Ok(rows) = db.list_downloads() else {
            return;
        };
        for row in rows {
            if row.state != states::MERGING {
                continue;
            }
            let cfg: DownloadConfig = serde_json::from_str(&row.config_json).unwrap_or_default();
            let ext = cfg
                .media_merge
                .as_ref()
                .and_then(|m| m.output_ext.clone())
                .unwrap_or_else(|| "mp4".into());
            let stem = row
                .final_name
                .rsplit_once('.')
                .map(|(s, _)| s.to_string())
                .unwrap_or_else(|| row.final_name.clone());
            let dest = std::path::Path::new(&row.dest_dir);
            if let Ok(entries) = std::fs::read_dir(dest) {
                let exact = format!("{stem}.{ext}.merging.part");
                let variant_prefix = format!("{stem}(");
                let variant_suffix = format!(".{ext}.merging.part");
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name == exact
                        || (name.starts_with(&variant_prefix) && name.ends_with(&variant_suffix))
                    {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
            let mut r = row.clone();
            r.state = states::FAILED.to_string();
            r.error_code = Some("MERGE_INTERRUPTED".into());
            r.error_message = Some("merge interrupted by restart; re-run the merge".into());
            if db.update_download(&r).is_ok() {
                info!(
                    id = row.id,
                    "startup sweep: stale MERGING row failed (MERGE_INTERRUPTED)"
                );
            }
        }
    }

    /// Whether the merge stage is available (API layers report capability honestly).
    pub fn has_media_merge(&self) -> bool {
        self.media_merge.lock().unwrap().is_some()
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

    /// MediaPipeline phase (review §9, docs/FFMPEG_INTEGRATION.md §2): merge two
    /// COMPLETED stream downloads (video + audio) into the final artifact via the
    /// injected `MediaMergePort` (`-c copy`, ffprobe-validated). The video row moves
    /// MERGING → COMPLETED and ends up describing the merged file; the audio row is
    /// left untouched. Cancel during MERGING kills the ffmpeg child (merge trigger).
    pub async fn merge_streams(&self, video_id: i64, audio_id: i64) -> Result<(), EngineError> {
        let port = self.media_merge.lock().unwrap().clone().ok_or_else(|| {
            EngineError::new(
                ErrorKind::InvalidConfig,
                "media pipeline not configured — set an LGPL ffmpeg integration first \
                     (docs/FFMPEG_INTEGRATION.md §3)",
            )
        })?;

        let video_row = self.completed_row(video_id, "video")?;
        let audio_row = self.completed_row(audio_id, "audio")?;
        let video_path = std::path::Path::new(&video_row.dest_dir).join(&video_row.final_name);
        let audio_path = std::path::Path::new(&audio_row.dest_dir).join(&audio_row.final_name);
        for (p, what) in [(&video_path, "video"), (&audio_path, "audio")] {
            if !p.is_file() {
                return Err(EngineError::new(
                    ErrorKind::InvalidConfig,
                    format!("{what} stream file is missing: {}", p.display()),
                ));
            }
        }

        // Output path: video's name with the merged extension (default mp4). If it
        // exists, pick `name(1).ext`, `name(2).ext`, … unless the row says overwrite.
        let cfg: DownloadConfig = serde_json::from_str(&video_row.config_json).unwrap_or_default();
        let ext = cfg
            .media_merge
            .as_ref()
            .and_then(|m| m.output_ext.clone())
            .unwrap_or_else(|| "mp4".into());
        let stem = video_row
            .final_name
            .rsplit_once('.')
            .map(|(s, _)| s.to_string())
            .unwrap_or_else(|| video_row.final_name.clone());
        let dest_dir = std::path::Path::new(&video_row.dest_dir).to_path_buf();
        let mut output = dest_dir.join(format!("{stem}.{ext}"));
        if !cfg.overwrite {
            let mut n = 1;
            while output.exists() {
                output = dest_dir.join(format!("{stem}({n}).{ext}"));
                n += 1;
            }
        }
        if output == video_path || output == audio_path {
            return Err(EngineError::new(
                ErrorKind::InvalidConfig,
                "merge output path would overwrite a source stream",
            ));
        }

        // Single-flight guard (audit P2): two concurrent merges for the same row
        // would both write the same output path.
        {
            let mut mg = self.merging.lock().unwrap();
            if mg.contains(&video_id) {
                return Err(EngineError::new(
                    ErrorKind::InvalidConfig,
                    format!("merge already in progress for id {video_id}"),
                ));
            }
            mg.insert(video_id);
        }

        // Write to a temp path and rename only on success (audit P0-2): a killed
        // or failed merge must never leave a truncated file at the destination.
        let part = dest_dir.join(format!(
            "{}.merging.part",
            output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));

        // Phase transition: video row becomes the merged-artifact row.
        let id = video_id;
        let mut row = video_row.clone();
        row.state = states::MERGING.to_string();
        if let Err(e) = self.db.update_download(&row) {
            self.merging.lock().unwrap().remove(&id);
            return Err(db_err(e));
        }
        self.events
            .send(crate::events::Event::DownloadState {
                id,
                state: states::MERGING.to_string(),
            })
            .ok();

        // Kill trigger registered for the phase duration; cancel() fires it.
        let (kill_tx, kill_rx) = tokio::sync::watch::channel(false);
        self.merge_triggers
            .lock()
            .unwrap()
            .insert(id, kill_tx.clone());

        let result = port
            .merge(id, &video_path, &audio_path, &part, &self.events, kill_rx)
            .await;

        self.merge_triggers.lock().unwrap().remove(&id);
        self.merging.lock().unwrap().remove(&id);

        // Finalize: rename the temp output only on success; on failure or cancel,
        // delete the residue so the out dir stays clean.
        let result = match result {
            Ok(()) => match std::fs::rename(&part, &output) {
                Ok(()) => Ok(()),
                Err(e) => Err(EngineError::new(
                    ErrorKind::Io,
                    format!("cannot finalize merged output: {e}"),
                )),
            },
            r @ Err(_) => r,
        };
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }

        match result {
            Ok(()) => {
                let size = std::fs::metadata(&output).ok().map(|m| m.len() as i64);
                row.state = states::COMPLETED.to_string();
                row.final_name = output
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| row.final_name.clone());
                row.temp_path = None;
                row.completed_at = Some(engine_state::now_iso());
                row.total_size = size;
                // Honest: the merged artifact is NOT the hash-verified stream — the
                // stream checksums do not describe this file. ffprobe validation is
                // the merge stage's own success criterion (FFMPEG_INTEGRATION §8).
                row.sha256 = None;
                row.blake3 = None;
                row.verified = false;
                self.db.update_download(&row).map_err(db_err)?;
                self.events
                    .send(crate::events::Event::Completed {
                        id,
                        path: output.display().to_string(),
                        size: size.unwrap_or(0) as u64,
                        sha256: None,
                        blake3: None,
                    })
                    .ok();
                Ok(())
            }
            Err(e) if e.kind == ErrorKind::Cancelled => {
                row.state = states::CANCELLED.to_string();
                row.error_code = Some("CANCELLED".into());
                row.error_message = Some("merge cancelled".into());
                let _ = self.db.update_download(&row);
                self.events
                    .send(crate::events::Event::DownloadState {
                        id,
                        state: states::CANCELLED.to_string(),
                    })
                    .ok();
                Err(e)
            }
            Err(e) => {
                row.state = states::FAILED.to_string();
                row.error_code = Some(e.kind.code().into());
                row.error_message = Some(e.message.clone());
                let _ = self.db.update_download(&row);
                self.events
                    .send(crate::events::Event::Failed {
                        id,
                        kind: e.kind.code().into(),
                        message: e.message.clone(),
                    })
                    .ok();
                Err(e)
            }
        }
    }

    fn completed_row(&self, id: i64, what: &str) -> Result<engine_state::DownloadRow, EngineError> {
        let row = self.db.get_download(id).map_err(db_err)?.ok_or_else(|| {
            EngineError::new(ErrorKind::InvalidConfig, format!("unknown {what} id {id}"))
        })?;
        if row.state != states::COMPLETED {
            return Err(EngineError::new(
                ErrorKind::InvalidConfig,
                format!(
                    "{what} stream (id {id}) is not completed (state: {})",
                    row.state
                ),
            ));
        }
        Ok(row)
    }

    pub async fn pause(&self, id: i64) -> Result<(), EngineError> {
        // merge phase is not pausable (ffmpeg has no resume) — honest error
        if let Ok(Some(row)) = self.db.get_download(id) {
            if row.state == states::MERGING {
                return Err(EngineError::new(
                    ErrorKind::InvalidConfig,
                    "merge in progress cannot be paused — cancel instead",
                ));
            }
        }
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
        // merge phase (review §9): a MERGING row has no download task — kill the
        // ffmpeg child via the trigger and wait for the phase to settle the row.
        let trigger = self.merge_triggers.lock().unwrap().get(&id).cloned();
        if let Some(tx) = trigger {
            let _ = tx.send(true);
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            loop {
                let st = self
                    .db
                    .get_download(id)
                    .ok()
                    .flatten()
                    .map(|r| r.state)
                    .unwrap_or_default();
                if st != states::MERGING {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    return Err(EngineError::new(
                        ErrorKind::Io,
                        "cancel timed out; merge phase still settling",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        self.remove_partial(id)
    }

    /// Remove the record (and part file, if any) entirely.
    /// A COMPLETED download is protected: its file has been finalized to the user's
    /// destination, so removing the row would orphan the file (no record, file still
    /// on disk). Callers must use an explicit "remove completed" semantic instead.
    /// Explicit removal that may target ANY state, including COMPLETED (unlike
    /// remove_partial, which protects completed rows). With `delete_file`, the
    /// final artifact is deleted BEFORE the record so a failed fs operation leaves
    /// the row in place to retry. The file is deleted first, record second, and a
    /// `removed` state event lets UIs drop the row immediately.
    pub async fn remove(&self, id: i64, delete_file: bool) -> Result<(), EngineError> {
        // never delete state under a live task (same guardrail as cancel, audit B1)
        if let Some(entry) = self.running_snapshot(id) {
            let _ = entry.cmd.send(TaskCommand::Cancel);
            if !wait_until_settled(self, id, Duration::from_secs(60)).await {
                return Err(EngineError::new(
                    ErrorKind::Io,
                    "remove timed out; task still settling",
                ));
            }
        }
        let row = self
            .db
            .get_download(id)
            .map_err(db_err)?
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidConfig, "unknown download id"))?;
        if delete_file && row.state == states::COMPLETED {
            let p = std::path::Path::new(&row.dest_dir).join(&row.final_name);
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(EngineError::new(
                        ErrorKind::Io,
                        format!("could not delete {}: {e}", p.display()),
                    ))
                }
            }
        }
        self.db.delete_download(id).map_err(db_err)?;
        if let Some(t) = row.temp_path {
            std::fs::remove_file(&t).ok(); // may already be renamed/absent
        }
        self.events
            .send(crate::events::Event::DownloadState {
                id,
                state: "removed".to_string(),
            })
            .ok();
        Ok(())
    }

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
            bandwidth_limit_bps: serde_json::from_str::<serde_json::Value>(&row.config_json)
                .ok()
                .and_then(|c| c.get("bandwidth_limit_bps").and_then(|v| v.as_u64())),
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
        // Cross-platform "unavailable destination": a path UNDER an existing FILE.
        // create_dir_all must fail on every OS (ENOTDIR / path-not-found). The old
        // /proc trick only worked on Linux — Windows happily creates C:\proc\... .
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let dest = blocker.join("sub");
        let e = DownloadManager::preflight_checked(&dest, Some(1024)).unwrap_err();
        assert!(
            e.message.contains("unavailable"),
            "must say 'unavailable', got: {}",
            e.message
        );
        assert!(!matches!(e.kind, ErrorKind::DiskFull));
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    struct NoHttp;
    #[async_trait::async_trait]
    impl crate::ports::HttpPort for NoHttp {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        async fn inspect(
            &self,
            _url: &str,
            _h: &crate::ports::RequestHeaders,
        ) -> Result<crate::inspect::ResourceInfo, EngineError> {
            Err(EngineError::new(ErrorKind::Io, "unused in merge tests"))
        }
        async fn open_range(
            &self,
            _r: crate::ports::RangeRequest,
        ) -> Result<crate::ports::RangeResponse, EngineError> {
            Err(EngineError::new(ErrorKind::Io, "unused in merge tests"))
        }
    }

    #[derive(Clone, Copy)]
    enum Mode {
        Ok,
        Fail,
        WriteThenFail,
        WaitCancel,
    }

    /// Fake merge stage: Ok writes the output (like ffmpeg would), Fail returns a
    /// merge failure, WaitCancel blocks until the kill trigger fires.
    struct FakeMerge(Mode);

    #[async_trait::async_trait]
    impl crate::ports::MediaMergePort for FakeMerge {
        async fn merge(
            &self,
            _id: i64,
            _video: &std::path::Path,
            _audio: &std::path::Path,
            output: &std::path::Path,
            _bus: &crate::events::EventBus,
            kill: tokio::sync::watch::Receiver<bool>,
        ) -> Result<(), EngineError> {
            match self.0 {
                Mode::Ok => {
                    std::fs::write(output, b"merged-bytes").map_err(|e| {
                        EngineError::new(ErrorKind::Io, format!("write failed: {e}"))
                    })?;
                    Ok(())
                }
                Mode::Fail => Err(EngineError::new(ErrorKind::MergeFailed, "boom")),
                Mode::WriteThenFail => {
                    // simulate ffmpeg dying mid-encode: bytes exist at the part path
                    let _ = std::fs::write(output, b"partial-merge-bytes");
                    Err(EngineError::new(
                        ErrorKind::MergeFailed,
                        "boom after partial write",
                    ))
                }
                Mode::WaitCancel => {
                    let mut kill = kill;
                    let _ = kill.wait_for(|v| *v).await;
                    Err(EngineError::new(ErrorKind::Cancelled, "merge cancelled"))
                }
            }
        }
    }

    #[tokio::test]
    async fn merge_failure_leaves_no_residue_and_fails_row() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::WriteThenFail))));

        let err = m.merge_streams(v, a).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::MergeFailed);
        let row = m.db.get_download(v).unwrap().unwrap();
        assert_eq!(row.state, states::FAILED);
        // no truncated output at the destination, no residue part file
        assert!(!dest.path().join("video.mp4").exists());
        let residue = std::fs::read_dir(dest.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().ends_with(".merging.part"));
        assert!(!residue, "merge residue leaked into the out dir");
    }

    #[tokio::test]
    async fn merge_single_flight_rejects_second_concurrent_merge() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::WaitCancel))));

        let m2 = m.clone();
        let first = tokio::spawn(async move { m2.merge_streams(v, a).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = m.merge_streams(v, a).await.unwrap_err();
        // Rejected honestly — either by the row-state check (row is already
        // MERGING) or by the in-memory single-flight guard.
        let msg = second.message.clone();
        assert!(
            msg.contains("not completed") || msg.contains("already in progress"),
            "{msg}"
        );
        // unwind the parked first merge via the cancel trigger
        let _ = m.cancel(v).await;
        let first_err = first.await.unwrap().unwrap_err();
        assert_eq!(first_err.kind, ErrorKind::Cancelled);
        // residue cleaned up on the cancel path
        assert!(!dest.path().join("video.mp4.merging.part").exists());
    }

    #[tokio::test]
    async fn stale_merging_row_swept_on_startup() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let mut row = m.db.get_download(v).unwrap().unwrap();
        row.state = states::MERGING.to_string();
        m.db.update_download(&row).unwrap();
        // residue from the pre-crash attempt: exact + numbered-collision variants
        std::fs::write(dest.path().join("video.mp4.merging.part"), b"half-written").unwrap();
        std::fs::write(
            dest.path().join("video(1).mp4.merging.part"),
            b"half-written-variant",
        )
        .unwrap();
        drop(m);

        // restart: the sweep must fail the row honestly and clean the residue
        let m2 = mgr(data.path());
        let row = m2.db.get_download(v).unwrap().unwrap();
        assert_eq!(row.state, states::FAILED);
        assert_eq!(row.error_code.as_deref(), Some("MERGE_INTERRUPTED"));
        assert!(!dest.path().join("video.mp4.merging.part").exists());
        assert!(!dest.path().join("video(1).mp4.merging.part").exists());
        assert!(dest.path().join("video.m4s").exists()); // source stream untouched
    }

    fn mgr(dir: &std::path::Path) -> Arc<DownloadManager> {
        let cfg = EngineConfig {
            data_dir: dir.to_path_buf(),
            ..Default::default()
        };
        DownloadManager::new(cfg, Arc::new(NoHttp)).unwrap()
    }

    fn completed_row(dest: &std::path::Path, name: &str, config_json: &str) -> DownloadRow {
        DownloadRow {
            url: "https://example.com/stream".into(),
            dest_dir: dest.display().to_string(),
            final_name: name.into(),
            state: states::COMPLETED.to_string(),
            accept_ranges: "yes".into(),
            mode: "segmented".into(),
            config_json: config_json.into(),
            created_at: engine_state::now_iso(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn merge_streams_happy_path_updates_video_row() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::Ok))));
        let mut rx = m.events();

        m.merge_streams(v, a).await.unwrap();

        let row = m.db.get_download(v).unwrap().unwrap();
        assert_eq!(row.state, states::COMPLETED);
        assert_eq!(row.final_name, "video.mp4");
        assert!(!row.verified);
        assert!(row.sha256.is_none());
        let out = dest.path().join("video.mp4");
        assert_eq!(std::fs::read(&out).unwrap(), b"merged-bytes");
        // audio row untouched
        assert_eq!(
            m.db.get_download(a).unwrap().unwrap().state,
            states::COMPLETED
        );
        // events: merging, merged (from the pipeline layer would come here — the fake
        // doesn't emit), download-state merging, completed
        let mut saw_merging = false;
        let mut saw_completed = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::DownloadState { state, .. } if state == states::MERGING => {
                    saw_merging = true
                }
                Event::Completed { path, .. } => {
                    assert_eq!(path, out.display().to_string());
                    saw_completed = true;
                }
                _ => {}
            }
        }
        assert!(saw_merging && saw_completed);
    }

    #[tokio::test]
    async fn merge_streams_without_pipeline_is_honest() {
        let data = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        let e = m.merge_streams(1, 2).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidConfig);
        assert!(e.message.contains("media pipeline not configured"));
    }

    #[tokio::test]
    async fn merge_failure_marks_row_failed() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::Fail))));
        let mut rx = m.events();
        let e = m.merge_streams(v, a).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::MergeFailed);
        let row = m.db.get_download(v).unwrap().unwrap();
        assert_eq!(row.state, states::FAILED);
        assert_eq!(row.error_code.as_deref(), Some("MERGE_FAILED"));
        let mut saw_failed = false;
        while let Ok(ev) = rx.try_recv() {
            if let Event::Failed { kind, .. } = ev {
                assert_eq!(kind, "MERGE_FAILED");
                saw_failed = true;
            }
        }
        assert!(saw_failed);
    }

    #[tokio::test]
    async fn cancel_during_merge_kills_and_forgets() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::WaitCancel))));

        let m2 = m.clone();
        let job = tokio::spawn(async move { m2.merge_streams(v, a).await });

        // wait for the phase transition
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let st = m.db.get_download(v).unwrap().unwrap().state;
            if st == states::MERGING {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "never reached MERGING"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        m.cancel(v).await.unwrap(); // cancel-and-forget
        let e = job.await.unwrap().unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert!(m.db.get_download(v).unwrap().is_none(), "row forgotten");
    }

    #[tokio::test]
    async fn existing_output_picks_numbered_name() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        std::fs::write(dest.path().join("video.mp4"), b"old").unwrap();
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", "{}"))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::Ok))));
        m.merge_streams(v, a).await.unwrap();
        assert_eq!(
            m.db.get_download(v).unwrap().unwrap().final_name,
            "video(1).mp4"
        );
    }

    #[tokio::test]
    async fn output_ext_from_media_merge_spec() {
        let data = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let m = mgr(data.path());
        std::fs::write(dest.path().join("video.m4s"), b"v").unwrap();
        std::fs::write(dest.path().join("audio.m4s"), b"a").unwrap();
        let cfg = r#"{"media_merge":{"audio_url":"https://example.com/a.m4a","output_ext":"mkv"}}"#;
        let v =
            m.db.insert_download(&completed_row(dest.path(), "video.m4s", cfg))
                .unwrap();
        let a =
            m.db.insert_download(&completed_row(dest.path(), "audio.m4s", "{}"))
                .unwrap();
        m.set_media_merge(Some(Arc::new(FakeMerge(Mode::Ok))));
        m.merge_streams(v, a).await.unwrap();
        assert_eq!(
            m.db.get_download(v).unwrap().unwrap().final_name,
            "video.mkv"
        );
    }
}
