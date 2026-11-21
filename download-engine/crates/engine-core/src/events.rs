//! Event bus: progress/state notifications (broadcast, lag-tolerant, never blocks the engine).

use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    DownloadState {
        id: i64,
        state: String,
    },
    Progress {
        id: i64,
        received: u64,
        total: Option<u64>,
        speed_bps: f64,
        connections: usize,
    },
    Restarting {
        id: i64,
        reason: String,
    },
    Completed {
        id: i64,
        path: String,
        size: u64,
        sha256: Option<String>,
        blake3: Option<String>,
    },
    Failed {
        id: i64,
        kind: String,
        message: String,
    },
    Verified {
        id: i64,
        ok: bool,
        algo: String,
    },
    /// Audit D4 (2026-11-21): resolve-phase feedback. URL resolution (Drive confirm
    /// walk, yt-dlp extraction) can take seconds to minutes before a row exists — the
    /// UI must be able to show that instead of a frozen spinner. Emitted with the
    /// ORIGINAL user URL (no download id exists yet).
    Resolving {
        url: String,
    },
    Resolved {
        url: String,
        note: Option<String>,
    },
}

pub type EventBus = broadcast::Sender<Event>;

pub fn make_bus(capacity: usize) -> (EventBus, broadcast::Receiver<Event>) {
    broadcast::channel(capacity.max(16))
}
