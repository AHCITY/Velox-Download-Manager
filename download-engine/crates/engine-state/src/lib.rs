//! Persistent download state: SQLite (WAL, synchronous=FULL) manifest of
//! downloads, segments, and bounded error history. See docs/RECOVERY_DESIGN.md.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// One persisted segment tuple: (path, etag, last_modified, last_segment_id).
pub type PersistedSegmentTuple = (String, String, String, Option<i64>);

pub mod states {
    pub const CREATED: &str = "created";
    /// waiting for an admission slot (start() returns immediately; the runner waits)
    pub const QUEUED: &str = "queued";
    pub const INSPECTING: &str = "inspecting";
    pub const DOWNLOADING: &str = "downloading";
    pub const PAUSED: &str = "paused";
    pub const VERIFYING: &str = "verifying";
    /// MediaPipeline (architecture review §9): the `-c copy` merge phase runs after
    /// both streams are complete. Not pausable; cancellable via the merge trigger.
    pub const MERGING: &str = "merging";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
}

#[derive(Debug, Clone, Default)]
pub struct DownloadRow {
    pub id: i64,
    pub url: String,
    pub resolved_url: Option<String>,
    pub dest_dir: String,
    pub temp_path: Option<String>,
    pub final_name: String,
    pub total_size: Option<i64>,
    pub etag: Option<String>,
    pub etag_is_weak: bool,
    pub last_modified: Option<String>,
    pub mime: Option<String>,
    pub content_encoding: Option<String>,
    pub accept_ranges: String, // yes | no | unknown
    pub mode: String,          // segmented | single | stream
    pub state: String,
    pub received: i64,
    pub max_connections: i64,
    pub config_json: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub sha256: Option<String>,
    pub blake3: Option<String>,
    pub verified: bool,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Source-provided preview image URL (yt-dlp metadata for media pages; any
    /// extractor that reports one). The UI shows it as the row/dossier art and
    /// falls back to the category icon when absent.
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SegmentRow {
    pub idx: i64,
    pub start_off: i64,
    pub end_off: Option<i64>, // inclusive; None = unbounded stream
    pub pos: i64,
    pub completed: bool,
}

#[derive(Debug, Clone)]
pub struct ErrorRow {
    pub code: String,
    pub message: String,
    pub class: String, // permanent | temporary | auth | restart
    pub segment_idx: Option<i64>,
}

const COLS: &str = "id, url, resolved_url, dest_dir, temp_path, final_name, total_size, etag, etag_is_weak, \
                    last_modified, mime, content_encoding, accept_ranges, mode, state, received, \
                    max_connections, config_json, error_code, error_message, sha256, blake3, verified, \
                    created_at, updated_at, completed_at, thumbnail";

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, StateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // the manifest holds auth config and URLs: enforce 0600 on Unix (audit A3)
        #[cfg(unix)]
        let conn = {
            use std::os::unix::fs::PermissionsExt;
            let c = Connection::open(path)?;
            if let Ok(md) = std::fs::metadata(path) {
                if md.permissions().mode() & 0o777 != 0o600 {
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
            }
            c
        };
        #[cfg(not(unix))]
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, StateError> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn path_hint(&self) -> &'static str {
        "sqlite (WAL, synchronous=FULL)"
    }

    fn configure(conn: &Connection) -> Result<(), StateError> {
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }

    fn migrate(conn: &Connection) -> Result<(), StateError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY, value TEXT NOT NULL
            );
            INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', '1');

            CREATE TABLE IF NOT EXISTS downloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                resolved_url TEXT,
                dest_dir TEXT NOT NULL,
                temp_path TEXT,
                final_name TEXT NOT NULL,
                total_size INTEGER,
                etag TEXT,
                etag_is_weak INTEGER NOT NULL DEFAULT 0,
                last_modified TEXT,
                mime TEXT,
                content_encoding TEXT,
                accept_ranges TEXT NOT NULL DEFAULT 'unknown',
                mode TEXT NOT NULL DEFAULT 'unknown',
                state TEXT NOT NULL,
                received INTEGER NOT NULL DEFAULT 0,
                max_connections INTEGER NOT NULL DEFAULT 1,
                config_json TEXT NOT NULL DEFAULT '{}',
                error_code TEXT,
                error_message TEXT,
                sha256 TEXT,
                blake3 TEXT,
                verified INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                thumbnail TEXT
            );

            CREATE TABLE IF NOT EXISTS segments (
                download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                idx INTEGER NOT NULL,
                start_off INTEGER NOT NULL,
                end_off INTEGER,
                pos INTEGER NOT NULL DEFAULT 0,
                completed INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (download_id, idx)
            );

            CREATE TABLE IF NOT EXISTS errors (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                ts TEXT NOT NULL,
                code TEXT NOT NULL,
                message TEXT NOT NULL,
                class TEXT NOT NULL,
                segment_idx INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_errors_dl ON errors(download_id);
            "#,
        )?;
        // migration: pre-thumbnail databases (0.7.0 and older) gain the column in place;
        // fresh databases already created it above.
        let has_col: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('downloads') WHERE name='thumbnail'",
            [],
            |r| r.get(0),
        )?;
        if has_col == 0 {
            conn.execute_batch("ALTER TABLE downloads ADD COLUMN thumbnail TEXT;")?;
        }
        Ok(())
    }

    /// Public timestamp helper (UTC ISO-8601, no chrono dependency).
    pub fn now() -> String {
        let d = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        iso8601(d.as_millis() as i64)
    }

    pub fn insert_download(&self, row: &DownloadRow) -> Result<i64, StateError> {
        let conn = self.conn.lock().unwrap();
        let ts = Self::now();
        conn.execute(
            &format!(
                "INSERT INTO downloads ({}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                COLS.trim_start_matches("id, ")
            ),
            params![
                row.url,
                row.resolved_url,
                row.dest_dir,
                row.temp_path,
                row.final_name,
                row.total_size,
                row.etag,
                row.etag_is_weak as i64,
                row.last_modified,
                row.mime,
                row.content_encoding,
                row.accept_ranges,
                row.mode,
                row.state,
                row.received,
                row.max_connections,
                row.config_json,
                row.error_code,
                row.error_message,
                row.sha256,
                row.blake3,
                row.verified as i64,
                if row.created_at.is_empty() { ts.clone() } else { row.created_at.clone() },
                ts,
                row.completed_at,
                row.thumbnail,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Replace every mutable column of the row (the orchestrator owns full state; simple + drift-free).
    pub fn update_download(&self, row: &DownloadRow) -> Result<(), StateError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET url=?, resolved_url=?, dest_dir=?, temp_path=?, final_name=?, total_size=?, \
             etag=?, etag_is_weak=?, last_modified=?, mime=?, content_encoding=?, accept_ranges=?, mode=?, \
             state=?, received=?, max_connections=?, config_json=?, error_code=?, error_message=?, sha256=?, \
             blake3=?, verified=?, updated_at=?, completed_at=?, thumbnail=? WHERE id=?",
            params![
                row.url,
                row.resolved_url,
                row.dest_dir,
                row.temp_path,
                row.final_name,
                row.total_size,
                row.etag,
                row.etag_is_weak as i64,
                row.last_modified,
                row.mime,
                row.content_encoding,
                row.accept_ranges,
                row.mode,
                row.state,
                row.received,
                row.max_connections,
                row.config_json,
                row.error_code,
                row.error_message,
                row.sha256,
                row.blake3,
                row.verified as i64,
                Self::now(),
                row.completed_at,
                row.thumbnail.clone(),
                row.id,
            ],
        )?;
        Ok(())
    }

    pub fn get_download(&self, id: i64) -> Result<Option<DownloadRow>, StateError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM downloads WHERE id=?"))?;
        let mut rows = stmt.query(params![id])?;
        Ok(rows.next()?.map(row_to_download).transpose()?)
    }

    pub fn list_downloads(&self) -> Result<Vec<DownloadRow>, StateError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM downloads ORDER BY id DESC LIMIT 1000"
        ))?;
        let rows = stmt.query_map([], row_to_download)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn delete_download(&self, id: i64) -> Result<(), StateError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM segments WHERE download_id=?", params![id])?;
        conn.execute("DELETE FROM errors WHERE download_id=?", params![id])?;
        conn.execute("DELETE FROM downloads WHERE id=?", params![id])?;
        Ok(())
    }

    pub fn set_segments(&self, download_id: i64, segs: &[SegmentRow]) -> Result<(), StateError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM segments WHERE download_id=?",
            params![download_id],
        )?;
        for s in segs {
            tx.execute(
                "INSERT INTO segments (download_id, idx, start_off, end_off, pos, completed) \
                 VALUES (?,?,?,?,?,?)",
                params![
                    download_id,
                    s.idx,
                    s.start_off,
                    s.end_off,
                    s.pos,
                    s.completed as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn segments(&self, download_id: i64) -> Result<Vec<SegmentRow>, StateError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT idx, start_off, end_off, pos, completed FROM segments WHERE download_id=? ORDER BY idx",
        )?;
        let rows = stmt.query_map(params![download_id], |r| {
            Ok(SegmentRow {
                idx: r.get(0)?,
                start_off: r.get(1)?,
                end_off: r.get(2)?,
                pos: r.get(3)?,
                completed: r.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Durable checkpoint: fsync of file data must have happened BEFORE this call (ADR-0007).
    pub fn checkpoint(
        &self,
        download_id: i64,
        updates: &[(i64, i64)],
        received: i64,
    ) -> Result<(), StateError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (idx, pos) in updates {
            tx.execute(
                "UPDATE segments SET pos=? WHERE download_id=? AND idx=? AND completed=0",
                params![pos, download_id, idx],
            )?;
        }
        tx.execute(
            "UPDATE downloads SET received=?, updated_at=? WHERE id=?",
            params![received, Self::now(), download_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Durable checkpoint: fsync of file data must have happened BEFORE this call (ADR-0007).
    /// Replaces the whole segment set (stolen splits included) and the aggregate atomically.
    pub fn persist_snapshot(
        &self,
        download_id: i64,
        segs: &[SegmentRow],
        received: i64,
    ) -> Result<(), StateError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM segments WHERE download_id=?",
            params![download_id],
        )?;
        for s in segs {
            tx.execute(
                "INSERT INTO segments (download_id, idx, start_off, end_off, pos, completed) VALUES (?,?,?,?,?,?)",
                params![download_id, s.idx, s.start_off, s.end_off, s.pos, s.completed as i64],
            )?;
        }
        tx.execute(
            "UPDATE downloads SET received=?, updated_at=? WHERE id=?",
            params![received, Self::now(), download_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn complete_segment(
        &self,
        download_id: i64,
        idx: i64,
        received: i64,
    ) -> Result<(), StateError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE segments SET completed=1, pos=COALESCE(end_off - start_off + 1, pos) \
             WHERE download_id=? AND idx=?",
            params![download_id, idx],
        )?;
        tx.execute(
            "UPDATE downloads SET received=?, updated_at=? WHERE id=?",
            params![received, Self::now(), download_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn record_error(&self, download_id: i64, err: &ErrorRow) -> Result<(), StateError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO errors (download_id, ts, code, message, class, segment_idx) VALUES (?,?,?,?,?,?)",
            params![download_id, Self::now(), err.code, err.message, err.class, err.segment_idx],
        )?;
        // keep last 50
        conn.execute(
            "DELETE FROM errors WHERE download_id=? AND id NOT IN \
             (SELECT id FROM errors WHERE download_id=? ORDER BY id DESC LIMIT 50)",
            params![download_id, download_id],
        )?;
        Ok(())
    }

    pub fn errors(&self, download_id: i64) -> Result<Vec<PersistedSegmentTuple>, StateError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT ts, code || ': ' || message, class, segment_idx FROM errors \
             WHERE download_id=? ORDER BY id DESC LIMIT 20",
        )?;
        let rows = stmt.query_map(params![download_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn find_same_destination(
        &self,
        url: &str,
        dest_dir: &str,
        final_name: &str,
    ) -> Result<Option<DownloadRow>, StateError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM downloads WHERE url=? AND dest_dir=? AND final_name=? ORDER BY id DESC LIMIT 1"
        ))?;
        let mut rows = stmt.query(params![url, dest_dir, final_name])?;
        Ok(rows.next()?.map(row_to_download).transpose()?)
    }
}

fn row_to_download(r: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRow> {
    Ok(DownloadRow {
        id: r.get(0).unwrap_or(0),
        url: r.get(1).unwrap_or_default(),
        resolved_url: r.get(2).unwrap_or_default(),
        dest_dir: r.get(3).unwrap_or_default(),
        temp_path: r.get(4).unwrap_or_default(),
        final_name: r.get(5).unwrap_or_default(),
        total_size: r.get(6).unwrap_or_default(),
        etag: r.get(7).unwrap_or_default(),
        etag_is_weak: r.get::<_, i64>(8).unwrap_or(0) != 0,
        last_modified: r.get(9).unwrap_or_default(),
        mime: r.get(10).unwrap_or_default(),
        content_encoding: r.get(11).unwrap_or_default(),
        accept_ranges: r.get(12).unwrap_or_default(),
        mode: r.get(13).unwrap_or_default(),
        state: r.get(14).unwrap_or_default(),
        received: r.get(15).unwrap_or_default(),
        max_connections: r.get(16).unwrap_or_default(),
        config_json: r.get(17).unwrap_or_default(),
        error_code: r.get(18).unwrap_or_default(),
        error_message: r.get(19).unwrap_or_default(),
        sha256: r.get(20).unwrap_or_default(),
        blake3: r.get(21).unwrap_or_default(),
        verified: r.get::<_, i64>(22).unwrap_or(0) != 0,
        created_at: r.get(23).unwrap_or_default(),
        updated_at: r.get(24).unwrap_or_default(),
        completed_at: r.get(25).unwrap_or_default(),
        thumbnail: r.get(26).unwrap_or_default(),
    })
}

/// Public UTC timestamp helper for other crates (no chrono dependency).
pub fn now_iso() -> String {
    Db::now()
}

/// Minimal ISO-8601 UTC formatter (no chrono dependency): epoch ms → "YYYY-MM-DDTHH:MM:SS.mmmZ".
fn iso8601(epoch_ms: i64) -> String {
    let secs = epoch_ms.div_euclid(1000);
    let ms = epoch_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
        ms
    )
}

/// Howard Hinnant's days→civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(state: &str) -> DownloadRow {
        DownloadRow {
            url: "https://example.com/a.bin".into(),
            dest_dir: "/tmp/x".into(),
            final_name: "a.bin".into(),
            state: state.into(),
            accept_ranges: "yes".into(),
            mode: "segmented".into(),
            total_size: Some(1000),
            max_connections: 4,
            ..Default::default()
        }
    }

    #[test]
    fn insert_update_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let mut row = sample(states::CREATED);
        let id = db.insert_download(&row).unwrap();
        row.id = id;
        row.state = states::DOWNLOADING.to_string();
        row.received = 512;
        row.etag = Some("\"abc\"".into());
        db.update_download(&row).unwrap();
        let got = db.get_download(id).unwrap().unwrap();
        assert_eq!(got.state, states::DOWNLOADING);
        assert_eq!(got.received, 512);
        assert_eq!(got.etag.as_deref(), Some("\"abc\""));
        assert!(got.updated_at.contains('T'));
    }

    #[test]
    fn segments_checkpoint_complete_flow() {
        let db = Db::open_in_memory().unwrap();
        let mut row = sample(states::DOWNLOADING);
        let id = db.insert_download(&row).unwrap();
        row.id = id;
        db.set_segments(
            id,
            &[
                SegmentRow {
                    idx: 0,
                    start_off: 0,
                    end_off: Some(499),
                    pos: 0,
                    completed: false,
                },
                SegmentRow {
                    idx: 1,
                    start_off: 500,
                    end_off: Some(999),
                    pos: 0,
                    completed: false,
                },
            ],
        )
        .unwrap();
        db.checkpoint(id, &[(0, 200)], 200).unwrap();
        let segs = db.segments(id).unwrap();
        assert_eq!(segs[0].pos, 200);
        db.complete_segment(id, 0, 500).unwrap();
        db.complete_segment(id, 1, 1000).unwrap();
        row.received = 1000;
        db.update_download(&row).unwrap();
        let segs = db.segments(id).unwrap();
        assert!(segs.iter().all(|s| s.completed));
        assert_eq!(segs[1].pos, 500); // end_off - start_off + 1
        let errs = db.errors(id).unwrap();
        assert!(errs.is_empty());
        db.record_error(
            id,
            &ErrorRow {
                code: "HTTP_429".into(),
                message: "rate limited".into(),
                class: "temporary".into(),
                segment_idx: Some(1),
            },
        )
        .unwrap();
        assert_eq!(db.errors(id).unwrap().len(), 1);
        db.delete_download(id).unwrap();
        assert!(db.get_download(id).unwrap().is_none());
        assert!(db.segments(id).unwrap().is_empty());
    }

    #[test]
    fn iso8601_known_values() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }
}
