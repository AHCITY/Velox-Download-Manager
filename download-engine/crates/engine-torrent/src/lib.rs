//! BitTorrent engine: magnet links + .torrent files via librqbit (Apache-2.0),
//! mapped onto Velox's download rows through the `TorrentPort` seam.
//!
//! Design:
//! - ONE librqbit Session per engine instance, spawned lazily on first use.
//!   Fastresume state lives under <data_dir>/torrents-session (survives restarts).
//! - The DESKTOP ENGINE stays the single source of truth: rows live in the same
//!   SQLite manifest as HTTP downloads; the torrent service only executes.
//! - TLS: librqbit is built with `default-features = false`; its reqwest unifies
//!   with the workspace's reqwest (rustls + ring provider, no aws-lc-sys) and the
//!   sha1 backend is forced to crypto-hash (OS APIs; no C build on Windows).
//! - Everything surfaces honest errors; a torrent failure never touches HTTP rows.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine_core::error::{EngineError, ErrorKind};
use tokio::sync::Mutex;

pub use engine_core::ports::{
    TorrentAdded, TorrentPort, TorrentStatsView, is_torrent_source as classify_torrent_source,
};

struct Inner {
    session: Arc<librqbit::Session>,
}

/// librqbit-backed implementation. Cloneable handle; the session is shared.
#[derive(Clone)]
pub struct TorrentService {
    inner: Arc<Mutex<Option<Arc<Inner>>>>,
    session_dir: PathBuf,
    default_out_dir: PathBuf,
}

impl TorrentService {
    /// Build the service. `session_dir` holds fastresume/persistence state;
    /// `default_out_dir` is the fallback destination when the row carries none.
    pub fn new(session_dir: PathBuf, default_out_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            session_dir,
            default_out_dir,
        }
    }

    /// The session, started on first use (DHT + trackers + fastresume
    /// persistence; no UPnP port mapping — firewalls just see outbound).
    async fn session(&self) -> Result<Arc<Inner>, EngineError> {
        ensure_ring_provider();
        let mut guard = self.inner.lock().await;
        if let Some(i) = guard.as_ref() {
            return Ok(i.clone());
        }
        let _ = std::fs::create_dir_all(&self.session_dir);
        let opts = librqbit::SessionOptions {
            // fastresume + periodic session dump: torrents continue where they
            // left off after an app restart (same crash-recovery contract as
            // the HTTP engine's segment checkpoints)
            fastresume: true,
            persistence: Some(librqbit::SessionPersistenceConfig::Json {
                // Velox keeps all engine state under its data dir
                folder: Some(self.session_dir.clone()),
            }),
            // listen on any free port; DHT enabled by default
            ..Default::default()
        };
        let session = librqbit::Session::new_with_opts(self.session_dir.clone(), opts)
            .await
            .map_err(|e| EngineError::new(ErrorKind::Io, format!("torrent session: {e}")))?;
        let inner = Arc::new(Inner { session });
        *guard = Some(inner.clone());
        Ok(inner)
    }

    /// Default destination for rows that carry no explicit out_dir.
    pub fn default_out_dir(&self) -> &Path {
        &self.default_out_dir
    }
}

/// librqbit builds its own reqwest client; the workspace selects
/// `rustls-no-provider` (ring only, no aws-lc-sys), so a process-level
/// provider must be installed before ANY client is built. First caller wins
/// (install is a no-op when one is already set) — engine-http installs the
/// same ring provider for the HTTP engine, so both share one decision.
fn ensure_ring_provider() {
    let provider = rustls::crypto::ring::default_provider();
    let _ = provider.install_default();
}

fn torrent_err(context: &str, e: impl std::fmt::Display) -> EngineError {
    EngineError::new(ErrorKind::Io, format!("{context}: {e}"))
}

#[async_trait::async_trait]
impl TorrentPort for TorrentService {
    async fn add(
        &self,
        source: &str,
        dest_dir: &Path,
        paused: bool,
    ) -> Result<TorrentAdded, EngineError> {
        let inner = self.session().await?;
        let _ = std::fs::create_dir_all(dest_dir);
        let mut opts = librqbit::AddTorrentOptions::default();
        opts.output_folder = Some(dest_dir.display().to_string());
        opts.paused = paused;
        // resume/seed over existing partial files (Velox rows are idempotent)
        opts.overwrite = true;
        // magnet:/http(s) URLs go straight through; local .torrent paths are
        // read and handed over as bytes (AddTorrent::from_url rejects paths)
        let at = if source.starts_with("magnet:")
            || source.starts_with("http://")
            || source.starts_with("https://")
        {
            librqbit::AddTorrent::from_url(source.to_string())
        } else {
            let bytes = std::fs::read(source).map_err(|e| {
                EngineError::new(
                    ErrorKind::Io,
                    format!("cannot read .torrent file {}: {e}", source),
                )
            })?;
            librqbit::AddTorrent::from_bytes(bytes)
        };
        let resp = inner
            .session
            .add_torrent(at, Some(opts))
            .await
            .map_err(|e| torrent_err("cannot add torrent", e))?;
        match resp {
            librqbit::AddTorrentResponse::AlreadyManaged(id, handle)
            | librqbit::AddTorrentResponse::Added(id, handle) => {
                // wait for metadata so the row can show name/size honestly
                // (magnets resolve via DHT/trackers; .torrent files are instant)
                let _ = handle.wait_until_initialized().await;
                let stats = handle.stats();
                Ok(TorrentAdded {
                    id: id as i64,
                    info_hash_hex: handle.info_hash().as_string(),
                    name: handle.name(),
                    total_bytes: Some(stats.total_bytes).filter(|t| *t > 0),
                })
            }
            librqbit::AddTorrentResponse::ListOnly(_) => Err(EngineError::new(
                ErrorKind::InvalidConfig,
                "torrent added in list-only mode; this is an engine bug",
            )),
        }
    }

    async fn stats(&self, id: i64) -> Result<Option<TorrentStatsView>, EngineError> {
        let inner = self.session().await?;
        let handle = inner.session.get(librqbit::api::TorrentIdOrHash::Id(id as usize));
        let Some(handle) = handle else { return Ok(None) };
        let stats = handle.stats();
        Ok(Some(TorrentStatsView {
            name: handle.name(),
            total_bytes: stats.total_bytes,
            progress_bytes: stats.progress_bytes,
            finished: stats.finished,
            paused: handle.is_paused(),
            error: stats.error,
        }))
    }

    async fn pause(&self, id: i64) -> Result<(), EngineError> {
        let inner = self.session().await?;
        let Some(handle) = inner.session.get(librqbit::api::TorrentIdOrHash::Id(id as usize)) else {
            return Ok(()); // already gone: pause is idempotent
        };
        let _ = handle;
        inner
            .session
            .pause(&handle)
            .await
            .map_err(|e| torrent_err("cannot pause torrent", e))
    }

    async fn unpause(&self, id: i64) -> Result<(), EngineError> {
        let inner = self.session().await?;
        let Some(handle) = inner.session.get(librqbit::api::TorrentIdOrHash::Id(id as usize)) else {
            return Err(EngineError::new(
                ErrorKind::InvalidConfig,
                "torrent is no longer in the session; re-add it",
            ));
        };
        inner
            .session
            .unpause(&handle)
            .await
            .map_err(|e| torrent_err("cannot resume torrent", e))
    }

    async fn remove(&self, id: i64, delete_files: bool) -> Result<(), EngineError> {
        let inner = self.session().await?;
        // delete() is a no-op when the id is unknown — exactly the idempotency
        // cancel() needs after an app restart.
        if inner
            .session
            .get(librqbit::api::TorrentIdOrHash::Id(id as usize))
            .is_none()
        {
            return Ok(()); // already gone: remove() is idempotent (cancel-after-restart)
        }
        inner
            .session
            .delete(
                librqbit::api::TorrentIdOrHash::Id(id as usize),
                delete_files,
            )
            .await
            .map_err(|e| torrent_err("cannot remove torrent", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_delegates_to_core() {
        assert!(classify_torrent_source("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862"));
        assert!(!classify_torrent_source("https://example.com/file.zip"));
    }

    #[tokio::test]
    async fn add_paused_metadata_only_needs_no_network() {
        use librqbit::create_torrent;

        // build a REAL .torrent via librqbit itself (validity guaranteed), then
        // add it paused: no tracker/DHT traffic, metadata read from the file
        let dir = tempfile::tempdir().unwrap();
        let payload_dir = dir.path().join("payload");
        std::fs::create_dir_all(&payload_dir).unwrap();
        std::fs::write(payload_dir.join("velox-sample.bin"), vec![0u8; 32768]).unwrap();

        let created = create_torrent(
            &payload_dir.join("velox-sample.bin"),
            librqbit::CreateTorrentOptions {
                name: Some("velox-sample.bin"),
                ..Default::default()
            },
            &librqbit::spawn_utils::BlockingSpawner::new(1),
        )
        .await
        .expect("create_torrent");
        let torrent_path = dir.path().join("sample.torrent");
        std::fs::write(&torrent_path, created.as_bytes().expect("serialize torrent")).unwrap();

        let service = TorrentService::new(dir.path().join("session"), dir.path().to_path_buf());
        let added = service
            .add(torrent_path.to_str().unwrap(), &dir.path().join("out"), true)
            .await
            .expect("paused add must not need the network");
        assert_eq!(added.name.as_deref(), Some("velox-sample.bin"));
        assert_eq!(added.total_bytes, Some(32768));
        let stats = service.stats(added.id).await.unwrap().expect("managed");
        assert_eq!(stats.total_bytes, 32768);
        assert_eq!(stats.progress_bytes, 0);
        assert!(!stats.finished);
        service.remove(added.id, true).await.expect("cleanup");
    }

    #[tokio::test]
    async fn remove_is_idempotent_and_pause_of_unknown_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let service = TorrentService::new(dir.path().join("session"), dir.path().to_path_buf());
        service.pause(424242).await.expect("pause unknown is ok");
        service.remove(424242, false).await.expect("remove unknown is ok");
    }
}
