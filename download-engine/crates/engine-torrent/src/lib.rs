//! BitTorrent + magnet download support for Velox (engine-torrent).
//!
//! Built on `librqbit` 9.x (Apache-2.0, pure Rust): DHT, trackers (HTTP/UDP),
//! uTP/TCP peers, magnet metadata resolution and .torrent files. The crate is
//! an adapter, not a protocol implementation: it maps Velox download rows
//! (`mode = "torrent"`) onto librqbit session handles and keeps the rows in the
//! SAME SQLite manifest as every other download — one list, one UI, pause /
//! resume / remove semantics preserved.
//!
//! Design notes:
//! - Rows are authoritative. librqbit session persistence is not used; instead
//!   the engine re-attaches incomplete torrent rows at startup (crash recovery
//!   follows the same "resume from what's on disk" contract as HTTP resuming).
//! - Progress flows through the ordinary event bus (`Progress`,
//!   `DownloadState`, `Completed`, `Failed`) so the web UI updates live without
//!   any torrent-specific code.
//! - `connections_active` on the wire is the live peer count; the speed is the
//!   swarm download rate.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use engine_core::error::{EngineError, ErrorKind};
use engine_core::events::{Event, EventBus};
use engine_core::ports::TorrentPort;
use engine_state::{states, Db};

use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions, TorrentStatsState};

// librqbit does not re-export the handle alias at the crate root; it is a plain
// Arc over the public ManagedTorrent type.
type ManagedTorrentHandle = Arc<librqbit::ManagedTorrent>;

type TorrentId = usize;

pub struct TorrentEngine {
    session: Arc<Session>,
    db: Arc<Db>,
    events: EventBus,
    /// row id → live handle (also the "is this row attached" marker).
    handles: Mutex<HashMap<i64, ManagedTorrentHandle>>,
    /// librqbit TorrentId → row id (for the poller's iteration).
    ids: Mutex<HashMap<TorrentId, i64>>,
}

impl TorrentEngine {
    /// Build the session and re-attach every incomplete torrent row (crash recovery).
    pub async fn new(data_dir: &Path, db: Arc<Db>, events: EventBus) -> Result<Arc<Self>, EngineError> {
        let session_dir = data_dir.join("torrents");
        std::fs::create_dir_all(&session_dir).map_err(io_err("cannot create torrent data dir"))?;
        let opts = SessionOptions {
            // DHT enabled (magnet resolution); trackers enabled; no persistence —
            // the Velox manifest is the single source of truth.
            persistence: None,
            fastresume: true,
            ..Default::default()
        };
        let session = Session::new_with_opts(session_dir, opts)
            .await
            .map_err(|e| EngineError::new(ErrorKind::Io, format!("torrent session failed to start: {e}")))?;
        let eng = Arc::new(Self {
            session,
            db,
            events,
            handles: Mutex::new(HashMap::new()),
            ids: Mutex::new(HashMap::new()),
        });
        eng.recover_rows().await;
        eng.spawn_poller();
        Ok(eng)
    }

    /// Re-attach rows left incomplete by a crash/upgrade (mode = torrent, not
    /// completed/failed/cancelled). A failed re-attach flips the row to FAILED
    /// with the real reason instead of leaving a zombie "downloading" row.
    async fn recover_rows(self: &Arc<Self>) {
        let rows = match self.db.list_downloads() {
            Ok(rows) => rows,
            Err(_) => return,
        };
        for row in rows {
            if row.mode != "torrent" {
                continue;
            }
            if matches!(row.state.as_str(), states::COMPLETED | states::CANCELLED) {
                continue;
            }
            let id = row.id;
            match self.attach(id, &row.url, Path::new(&row.dest_dir), true).await {
                Ok(_) => {
                    let _ = self.events.send(Event::DownloadState {
                        id,
                        state: states::PAUSED.into(),
                    });
                }
                Err(e) => {
                    warn_row_fail(&self.db, &self.events, id, &e.message);
                }
            }
        }
    }

    /// Attach a source (magnet URI / .torrent path / http URL) to a row.
    async fn attach(
        &self,
        id: i64,
        source: &str,
        out_dir: &Path,
        paused: bool,
    ) -> Result<ManagedTorrentHandle, EngineError> {
        // absolute output folder (librqbit requires one)
        let out_dir = std::fs::canonicalize(out_dir)
            .or_else(|_| {
                std::fs::create_dir_all(out_dir).map(|_| out_dir.to_path_buf())
            })
            .map_err(io_err("torrent output folder unavailable"))?;

        let bytes_or_none = if !source.starts_with("magnet:")
            && !source.starts_with("http://")
            && !source.starts_with("https://")
        {
            // local .torrent file: read the bytes (validated: exists + size cap)
            let p = Path::new(source);
            let md = std::fs::metadata(p).map_err(|_| {
                EngineError::new(
                    ErrorKind::InvalidUrl,
                    format!("torrent file not found: {source}"),
                )
            })?;
            if md.len() > 4 * 1024 * 1024 {
                return Err(EngineError::new(
                    ErrorKind::InvalidUrl,
                    "torrent file is unreasonably large (>4 MiB); refusing",
                ));
            }
            Some(std::fs::read(p).map_err(io_err("cannot read torrent file"))?)
        } else {
            None
        };

        let add = match bytes_or_none {
            Some(bytes) => AddTorrent::from_bytes(bytes),
            None => AddTorrent::from_url(source.to_string()),
        };
        let opts = AddTorrentOptions {
            output_folder: Some(out_dir.display().to_string()),
            overwrite: true, // resume from existing pieces on re-add
            paused,
            ..Default::default()
        };
        let resp = self
            .session
            .add_torrent(add, Some(opts))
            .await
            .map_err(|e| EngineError::new(ErrorKind::InvalidUrl, format!("torrent could not be added: {e}")))?;
        let handle = resp
            .into_handle()
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidUrl, "torrent source was list-only; nothing to download"))?;
        let tid = handle.id();
        self.handles.lock().unwrap().insert(id, handle.clone());
        self.ids.lock().unwrap().insert(tid, id);
        Ok(handle)
    }

    fn spawn_poller(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                let Some(this) = this.upgrade() else { return };
                tick += 1;
                this.poll_once(tick).await;
            }
        });
    }

    /// One poll pass: sync handle stats into the rows (events every tick, DB
    /// every 2nd tick — WAL+FULL commits are too expensive for 1 Hz).
    async fn poll_once(self: &Arc<Self>, tick: u64) {
        let pairs: Vec<(i64, ManagedTorrentHandle)> = self
            .handles
            .lock()
            .unwrap()
            .iter()
            .map(|(id, h)| (*id, h.clone()))
            .collect();
        for (id, handle) in pairs {
            let stats = handle.stats();
            let total = stats.total_bytes;
            let received = stats.progress_bytes.min(total.max(1));
            let (speed, peers) = stats
                .live
                .as_ref()
                .map(|l| (l.download_speed.as_bytes(), l.snapshot.peer_stats.live as usize))
                .unwrap_or((0, 0));

            let state = stats_state_of(&stats.state, stats.finished);
            let _ = self.events.send(Event::Progress {
                id,
                received,
                total: Some(total),
                speed_bps: speed as f64,
                connections: peers,
            });

            // metadata often arrives only after the magnet resolves
            let mut want_db = tick % 2 == 0;
            if let Some(name) = handle.name() {
                if let Ok(Some(mut row)) = self.db.get_download(id) {
                    let current = sanitize_torrent_name(&row.final_name);
                    let candidate = sanitize_torrent_name(&name);
                    if !candidate.is_empty() && candidate != current && looks_like_placeholder(&row.final_name) {
                        row.final_name = candidate;
                        if self.db.update_download(&row).is_ok() {
                            want_db = false; // already persisted this tick
                        }
                    }
                }
            }
            if want_db {
                self.sync_row(id, handle.clone(), total, received).await;
            }
        }
    }

    /// DB sync + state transitions for one torrent row.
    async fn sync_row(&self, id: i64, handle: ManagedTorrentHandle, total: u64, received: u64) {
        let Ok(Some(mut row)) = self.db.get_download(id) else { return };
        let stats = handle.stats();
        let new_state = stats_state_of(&stats.state, stats.finished);
        let old_state = row.state.clone();

        row.total_size = Some(total as i64);
        row.received = received as i64;
        row.state = new_state.clone();

        if let Some(err) = &stats.error {
            row.error_code = Some("TORRENT".into());
            row.error_message = Some(err.clone());
        } else if new_state != old_state {
            row.error_code = None;
            row.error_message = None;
        }
        if stats.finished {
            row.completed_at = row.completed_at.or_else(|| Some(engine_state::now_iso()));
        }
        let _ = self.db.update_download(&row);

        if new_state != old_state {
            let _ = self.events.send(Event::DownloadState {
                id,
                state: new_state.clone(),
            });
            if new_state == states::COMPLETED {
                let path = std::path::Path::new(&row.dest_dir)
                    .join(handle.output_folder().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let _ = self.events.send(Event::Completed {
                    id,
                    path: path.display().to_string(),
                    size: total,
                    sha256: None,
                    blake3: None,
                });
            }
        }
    }
}

fn stats_state_of(state: &TorrentStatsState, finished: bool) -> String {
    match state {
        TorrentStatsState::Initializing { .. } => {
            if finished { states::COMPLETED.into() } else { states::DOWNLOADING.into() }
        }
        TorrentStatsState::Live => {
            if finished { states::COMPLETED.into() } else { states::DOWNLOADING.into() }
        }
        TorrentStatsState::Paused => {
            if finished { states::COMPLETED.into() } else { states::PAUSED.into() }
        }
        TorrentStatsState::Error => states::FAILED.into(),
    }
}

fn io_err(what: &'static str) -> impl Fn(std::io::Error) -> EngineError {
    move |e| EngineError::new(ErrorKind::Io, format!("{what}: {e}"))
}

fn looks_like_placeholder(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.is_empty()
        || n.starts_with("torrent ")
        || n.starts_with("magnet")
        || n.ends_with(".torrent")
        || n.starts_with("download")
}

fn sanitize_torrent_name(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .trim()
        .chars()
        .take(180)
        .collect()
}

fn warn_row_fail(db: &Arc<Db>, events: &EventBus, id: i64, message: &str) {
    if let Ok(Some(mut row)) = db.get_download(id) {
        row.state = states::FAILED.into();
        row.error_code = Some("TORRENT".into());
        row.error_message = Some(message.to_string());
        let _ = db.update_download(&row);
    }
    let _ = events.send(Event::Failed {
        id,
        kind: "TORRENT".into(),
        message: message.to_string(),
    });
}

#[async_trait]
impl TorrentPort for TorrentEngine {
    async fn add(&self, id: i64, source: &str, out_dir: &Path) -> Result<(), EngineError> {
        let handle = self.attach(id, source, out_dir, false).await?;
        // kick the state into "downloading" so the UI sees progress right away
        self.sync_row(id, handle, 0, 0).await;
        Ok(())
    }

    async fn pause(&self, id: i64) -> Result<(), EngineError> {
        let handle = self
            .handles
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidConfig, "torrent is not attached"))?;
        self.session
            .pause(&handle)
            .await
            .map_err(|e| EngineError::new(ErrorKind::Io, format!("torrent pause failed: {e}")))?;
        self.sync_row(id, handle, 0, 0).await;
        Ok(())
    }

    async fn resume(&self, id: i64) -> Result<(), EngineError> {
        let handle = self
            .handles
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| EngineError::new(ErrorKind::InvalidConfig, "torrent is not attached"))?;
        if let Err(e) = self.session.unpause(&handle).await {
            // resume-after-crash and "Retry" hit an already-live torrent — that
            // is the happy path, not an error
            let msg = e.to_string();
            if !msg.to_ascii_lowercase().contains("already") && !msg.to_ascii_lowercase().contains("live") {
                return Err(EngineError::new(ErrorKind::Io, format!("torrent resume failed: {e}")));
            }
        }
        self.sync_row(id, handle, 0, 0).await;
        Ok(())
    }

    async fn remove(&self, id: i64, delete_files: bool) -> Result<(), EngineError> {
        let handle = self.handles.lock().unwrap().remove(&id);
        if let Some(h) = handle {
            let tid = h.id();
            self.ids.lock().unwrap().remove(&tid);
            let _ = self
                .session
                .delete(librqbit::api::TorrentIdOrHash::Id(tid), delete_files)
                .await;
        }
        Ok(())
    }
}
