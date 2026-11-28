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
//!   `DownloadState`, `Completed`, `Failed`) so the UI updates live without
//!   any torrent-specific code.
//! - `connections_active` on the wire is the live peer count; the speed is the
//!   swarm download rate.
//! - Magnet adds never block: `session.add_torrent` on a metadata-less magnet
//!   waits for the swarm to deliver the metadata, which can take forever when
//!   no seeder answers. Attach runs in a bounded, cancellable resolver task so
//!   an unresolvable magnet can never stall `download.create` or app startup.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// How long a magnet may wait for the swarm to deliver metadata before the
/// row fails with an honest error.
const MAGNET_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub struct TorrentEngine {
    session: Arc<Session>,
    db: Arc<Db>,
    events: EventBus,
    /// row id → live handle (also the "is this row attached" marker).
    handles: Mutex<HashMap<i64, ManagedTorrentHandle>>,
    /// row id → resolver cancel flag (magnet metadata still resolving).
    resolving: Mutex<HashMap<i64, Arc<AtomicBool>>>,
    /// Self weak pointer: lets `&self` trait methods spawn tasks that keep the
    /// engine alive without creating reference cycles at construction.
    self_weak: Mutex<std::sync::Weak<TorrentEngine>>,
}

impl TorrentEngine {
    /// Build the session and re-attach every incomplete torrent row (crash recovery).
    pub async fn new(
        data_dir: &Path,
        db: Arc<Db>,
        events: EventBus,
    ) -> Result<Arc<Self>, EngineError> {
        let session_dir = data_dir.join("torrents");
        std::fs::create_dir_all(&session_dir).map_err(io_err("cannot create torrent data dir"))?;
        let listen = listen_options();
        let opts = SessionOptions {
            // DHT enabled (magnet resolution); trackers enabled; no persistence —
            // the Velox manifest is the single source of truth.
            persistence: None,
            fastresume: true,
            // Accept inbound peer connections (seeding + swarm health). Without
            // this the client can only connect OUT and behaves like a leech-only
            // client. Default port 0 = ephemeral → several Velox instances can
            // run side-by-side; set DXDL_TORRENT_PORT to a fixed port for NAT
            // port-forwarding (UPnP runs by default, DXDL_TORRENT_UPNP=0 opt-out).
            listen: Some(listen),
            ..Default::default()
        };
        let session = Session::new_with_opts(session_dir, opts)
            .await
            .map_err(|e| {
                EngineError::new(
                    ErrorKind::Io,
                    format!("torrent session failed to start: {e}"),
                )
            })?;
        let eng = Arc::new_cyclic(|weak| Self {
            session,
            db,
            events,
            handles: Mutex::new(HashMap::new()),
            resolving: Mutex::new(HashMap::new()),
            self_weak: Mutex::new(weak.clone()),
        });
        eng.recover_rows().await;
        eng.spawn_poller();
        Ok(eng)
    }

    fn weak(&self) -> Arc<TorrentEngine> {
        self.self_weak
            .lock()
            .unwrap()
            .upgrade()
            .expect("engine alive while its methods run")
    }

    /// Re-attach rows left incomplete by a crash/upgrade (mode = torrent, not
    /// completed/cancelled). Recovery never blocks the host: file-backed rows
    /// attach inline (metadata is known); magnets resolve in a background task.
    async fn recover_rows(self: &Arc<Self>) {
        let rows = match self.db.list_downloads() {
            Ok(rows) => rows,
            Err(_) => return,
        };
        for row in rows {
            if row.mode != "torrent"
                || matches!(row.state.as_str(), states::COMPLETED | states::CANCELLED)
            {
                continue;
            }
            let id = row.id;
            let source = row.url.clone();
            let out_dir = row.dest_dir.clone();
            if source.starts_with("magnet:") {
                // background resolver; the row stays `created` until metadata lands
                self.spawn_magnet_resolver(id, source, out_dir, true);
            } else {
                match self
                    .attach_inline(id, &source, Path::new(&out_dir), true)
                    .await
                {
                    Ok(_) => {
                        let _ = self.events.send(Event::DownloadState {
                            id,
                            state: states::PAUSED.into(),
                        });
                    }
                    Err(e) => warn_row_fail(&self.db, &self.events, id, &e.message),
                }
            }
        }
    }

    /// Read a local .torrent source (validated: exists + size cap).
    fn read_torrent_file(source: &str) -> Result<Vec<u8>, EngineError> {
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
        std::fs::read(p).map_err(io_err("cannot read torrent file"))
    }

    /// Attach a metadata-KNOWN source (.torrent bytes / http URL) to a row.
    /// Fast: no swarm round-trip, safe to await inline.
    async fn attach_inline(
        &self,
        id: i64,
        source: &str,
        out_dir: &Path,
        paused: bool,
    ) -> Result<ManagedTorrentHandle, EngineError> {
        let out_dir = std::fs::canonicalize(out_dir)
            .or_else(|_| std::fs::create_dir_all(out_dir).map(|_| out_dir.to_path_buf()))
            .map_err(io_err("torrent output folder unavailable"))?;

        // magnets and http(s) URLs go through librqbit's URL path (magnets resolve
        // via DHT/trackers, URLs are fetched); local files load as raw bytes
        let add = if source.starts_with("http://") || source.starts_with("https://") {
            AddTorrent::from_url(source.to_string())
        } else if source.starts_with("magnet:") {
            AddTorrent::from_url(source.to_string())
        } else {
            AddTorrent::from_bytes(Self::read_torrent_file(source)?)
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
            .map_err(|e| {
                EngineError::new(
                    ErrorKind::InvalidUrl,
                    format!("torrent could not be added: {e}"),
                )
            })?;
        let handle = resp.into_handle().ok_or_else(|| {
            EngineError::new(
                ErrorKind::InvalidUrl,
                "torrent source was list-only; nothing to download",
            )
        })?;
        self.handles.lock().unwrap().insert(id, handle.clone());
        self.sync_row(id, handle.clone(), 0, 0).await;
        Ok(handle)
    }

    /// Resolve a magnet in the background (bounded): the row stays `created`
    /// until the swarm delivers the metadata, then the poller takes over. The
    /// task aborts when the row is paused/removed/cancelled meanwhile.
    fn spawn_magnet_resolver(&self, id: i64, source: String, out_dir: String, paused: bool) {
        let this = self.weak();
        let cancel = Arc::new(AtomicBool::new(false));
        self.resolving.lock().unwrap().insert(id, cancel.clone());
        tokio::spawn(async move {
            let Some(this) = this.self_weak.lock().unwrap().upgrade() else {
                return;
            };
            let attempt = this.attach_inline(id, &source, Path::new(&out_dir), paused);
            let res = tokio::time::timeout(MAGNET_RESOLVE_TIMEOUT, attempt).await;
            this.resolving.lock().unwrap().remove(&id);
            if cancel.load(Ordering::SeqCst) {
                return; // paused/removed while resolving: the caller owns the row state
            }
            let msg = match res {
                Ok(Ok(_)) => return, // attached; the poller drives the row from here
                Ok(Err(e)) => e.message,
                Err(_) => format!(
                    "magnet metadata did not resolve in {} s — no peers answered. \
                     Check your network or retry later (the swarm may be empty).",
                    MAGNET_RESOLVE_TIMEOUT.as_secs()
                ),
            };
            warn_row_fail(&this.db, &this.events, id, &msg);
        });
    }

    /// True when the row's magnet is still resolving in the background.
    fn is_resolving(&self, id: i64) -> bool {
        self.resolving.lock().unwrap().contains_key(&id)
    }

    fn cancel_resolver(&self, id: i64) {
        if let Some(flag) = self.resolving.lock().unwrap().remove(&id) {
            flag.store(true, Ordering::SeqCst);
        }
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
                .map(|l| {
                    (
                        l.download_speed.as_bytes(),
                        l.snapshot.peer_stats.live as usize,
                    )
                })
                .unwrap_or((0, 0));

            let _ = self.events.send(Event::Progress {
                id,
                received,
                total: Some(total),
                speed_bps: speed as f64,
                connections: peers,
            });

            // metadata often arrives only after the magnet resolves
            let mut want_db = tick.is_multiple_of(2);
            if let Some(name) = handle.name() {
                if let Ok(Some(mut row)) = self.db.get_download(id) {
                    let current = sanitize_torrent_name(&row.final_name);
                    let candidate = sanitize_torrent_name(&name);
                    if !candidate.is_empty()
                        && candidate != current
                        && looks_like_placeholder(&row.final_name)
                    {
                        row.final_name = candidate;
                        if self.db.update_download(&row).is_ok() {
                            want_db = false; // already persisted this tick
                        }
                    }
                }
            }
            if want_db {
                self.sync_row(id, handle, total, received).await;
            }
        }
    }

    /// DB sync + state transitions for one torrent row.
    async fn sync_row(&self, id: i64, handle: ManagedTorrentHandle, total: u64, received: u64) {
        let Ok(Some(mut row)) = self.db.get_download(id) else {
            return;
        };
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
                let path = std::path::Path::new(&row.dest_dir).join(
                    handle
                        .output_folder()
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
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
            if finished {
                states::COMPLETED.into()
            } else {
                states::DOWNLOADING.into()
            }
        }
        TorrentStatsState::Live => {
            if finished {
                states::COMPLETED.into()
            } else {
                states::DOWNLOADING.into()
            }
        }
        TorrentStatsState::Paused => {
            if finished {
                states::COMPLETED.into()
            } else {
                states::PAUSED.into()
            }
        }
        TorrentStatsState::Error => states::FAILED.into(),
    }
}

fn io_err(what: &'static str) -> impl Fn(std::io::Error) -> EngineError {
    move |e| EngineError::new(ErrorKind::Io, format!("{what}: {e}"))
}

/// Listener options for the librqbit session.
///
/// `DXDL_TORRENT_PORT` — fixed listen port (uTorrent-style "incoming port").
/// Unset or 0 binds an EPHEMERAL port so several Velox instances can run
/// side-by-side; each announces the port it actually bound. UPnP port
/// forwarding is on by default (matches uTorrent/BitTorrent default), the
/// announce port follows the bound port. `DXDL_TORRENT_UPNP=0` opts out.
fn listen_options() -> librqbit::ListenerOptions {
    let port = torrent_listen_port();
    let upnp = std::env::var("DXDL_TORRENT_UPNP")
        .ok()
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true);
    librqbit::ListenerOptions {
        listen_addr: (std::net::Ipv6Addr::UNSPECIFIED, port).into(),
        enable_upnp_port_forwarding: upnp,
        ..Default::default()
    }
}

fn torrent_listen_port() -> u16 {
    std::env::var("DXDL_TORRENT_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_listen_port_env_overrides_ephemeral_default() {
        // unset → ephemeral (0)
        std::env::remove_var("DXDL_TORRENT_PORT");
        assert_eq!(torrent_listen_port(), 0);
        // garbage → ephemeral (0), honest fallback
        std::env::set_var("DXDL_TORRENT_PORT", "not-a-port");
        assert_eq!(torrent_listen_port(), 0);
        // explicit port honored
        std::env::set_var("DXDL_TORRENT_PORT", "4240");
        assert_eq!(torrent_listen_port(), 4240);
        std::env::remove_var("DXDL_TORRENT_PORT");
    }
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
        if source.starts_with("magnet:") {
            // never block: metadata resolution happens in the background task
            self.spawn_magnet_resolver(
                id,
                source.to_string(),
                out_dir.display().to_string(),
                false,
            );
            return Ok(());
        }
        self.attach_inline(id, source, out_dir, false).await?;
        Ok(())
    }

    async fn pause(&self, id: i64) -> Result<(), EngineError> {
        let handle = self.handles.lock().unwrap().get(&id).cloned();
        match handle {
            Some(handle) => {
                self.session.pause(&handle).await.map_err(|e| {
                    EngineError::new(ErrorKind::Io, format!("torrent pause failed: {e}"))
                })?;
                self.sync_row(id, handle, 0, 0).await;
            }
            None => {
                // still resolving: abort the resolver, the row keeps its state
                self.cancel_resolver(id);
            }
        }
        Ok(())
    }

    async fn resume(&self, id: i64) -> Result<(), EngineError> {
        let handle = self.handles.lock().unwrap().get(&id).cloned();
        match handle {
            Some(handle) => {
                if let Err(e) = self.session.unpause(&handle).await {
                    // resume-after-crash and "Retry" hit an already-live torrent —
                    // that is the happy path, not an error
                    let msg = e.to_string();
                    let low = msg.to_ascii_lowercase();
                    if !low.contains("already") && !low.contains("live") {
                        return Err(EngineError::new(
                            ErrorKind::Io,
                            format!("torrent resume failed: {e}"),
                        ));
                    }
                }
                self.sync_row(id, handle, 0, 0).await;
            }
            None => {
                // never attached (crash recovery / failed resolve): re-attach.
                // A magnet re-resolves in the background; a .torrent attaches inline.
                let Ok(Some(row)) = self.db.get_download(id) else {
                    return Err(EngineError::new(
                        ErrorKind::InvalidConfig,
                        "unknown download id",
                    ));
                };
                if self.is_resolving(id) {
                    return Ok(()); // already resolving
                }
                if row.url.starts_with("magnet:") {
                    self.spawn_magnet_resolver(id, row.url.clone(), row.dest_dir.clone(), false);
                } else {
                    self.attach_inline(id, &row.url, Path::new(&row.dest_dir), false)
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn remove(&self, id: i64, delete_files: bool) -> Result<(), EngineError> {
        // abort any in-flight resolver BEFORE detaching the handle
        self.cancel_resolver(id);
        let handle = self.handles.lock().unwrap().remove(&id);
        if let Some(h) = handle {
            let tid = h.id();
            let _ = self
                .session
                .delete(librqbit::api::TorrentIdOrHash::Id(tid), delete_files)
                .await;
        }
        Ok(())
    }
}
