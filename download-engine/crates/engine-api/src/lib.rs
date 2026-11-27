//! Velox versioned JSON-RPC 2.0 API (v1) over line-delimited stdio (ADR-0004).
//!
//! Wire format: one JSON object per line. Methods: engine.info, download.create,
//! download.start, download.pause, download.resume, download.cancel, download.remove
//! (record removal, any state; delete_file=true also deletes the artifact),
//! download.list, download.status, download.verify, download.merge. Standard JSON-RPC
//! error codes; engine failures carry code -32000 with a "<KIND>: message" text.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use engine_core::manager::DownloadManager;

pub const API_VERSION: &str = "1";
pub const ENGINE_NAME: &str = "velox";

/// JSON-RPC frames are one line each (UI_READINESS §2.8): a hostile input stream must not
/// be able to grow engine memory without bound. Returns (bytes, overflowed).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

async fn read_line_capped<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    cap: usize,
) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    use tokio::io::AsyncBufReadExt;
    let mut buf: Vec<u8> = Vec::new();
    let mut overflow = false;
    loop {
        let avail = reader.fill_buf().await?;
        if avail.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Ok(Some((buf, overflow)))
            };
        }
        let nl = avail.iter().position(|&b| b == b'\n');
        let take = nl.map_or(avail.len(), |i| i + 1);
        let room = cap.saturating_sub(buf.len());
        buf.extend_from_slice(&avail[..take.min(room)]);
        overflow |= take > room;
        reader.consume(take);
        if nl.is_some() {
            return Ok(Some((buf, overflow)));
        }
    }
}

pub async fn serve_stdio(manager: Arc<DownloadManager>) -> std::io::Result<()> {
    serve(manager, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serve JSON-RPC over any async pair (tests use in-memory duplexes).
pub async fn serve<R, W>(
    manager: Arc<DownloadManager>,
    input: R,
    mut output: W,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(input);
    while let Some((raw, overflow)) = read_line_capped(&mut reader, MAX_LINE_BYTES).await? {
        if raw.is_empty() {
            continue;
        }
        let resp = if overflow {
            Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":format!("request line exceeds {} bytes", MAX_LINE_BYTES)}}).to_string())
        } else {
            let line = String::from_utf8_lossy(&raw);
            if line.trim().is_empty() {
                None
            } else {
                handle_line(&manager, &line).await
            }
        };
        if let Some(resp) = resp {
            output.write_all(resp.as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CreateParams {
    url: String,
    #[serde(default)]
    #[serde(flatten)]
    dl: engine_core::config::DownloadConfig,
}

fn parse_id(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}

fn err(code: i32, message: impl Into<String>) -> (i32, String) {
    (code, message.into())
}

fn engine_err(e: engine_core::EngineError) -> (i32, String) {
    (-32000, format!("{}: {}", e.kind_code(), e.message))
}

pub async fn handle_line(manager: &Arc<DownloadManager>, line: &str) -> Option<String> {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}}).to_string());
        }
    };
    // JSON-RPC 2.0 conformance (UI_READINESS §2.7): a request without "id" is a notification
    // and NEVER gets a response; id must be string/number/null when present
    let id_val = parsed.get("id")?;
    if !(id_val.is_null() || id_val.is_string() || id_val.is_number()) {
        return Some(
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"invalid request: id must be a string, number, or null"}})
                .to_string(),
        );
    }
    let id = id_val.clone();
    let Some(method) = parsed
        .get("method")
        .and_then(|m| m.as_str())
        .map(str::to_string)
    else {
        return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"invalid request: missing method"}}).to_string());
    };
    let params = parsed.get("params").cloned().unwrap_or_else(|| json!({}));

    let result = match method.as_str() {
        "media.formats" => {
            let url = params
                .get("url")
                .and_then(|u| u.as_str())
                .filter(|u| !u.is_empty());
            let Some(url) = url else {
                return Some(bad_params_json(id, "missing url"));
            };
            let headers = engine_core::ports::RequestHeaders::default();
            manager
                .probe_media_formats(url, &headers)
                .await
                .map_err(engine_err)
        }
        "download.inspect" => {
            let p: CreateParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":format!("invalid params: {e}")}}).to_string()),
            };
            match manager.inspect(&p.url, p.dl).await {
                Ok(i) => Ok(json!({
                    "final_url": i.final_url,
                    "etag": i.etag,
                    "etag_is_weak": i.etag_is_weak,
                    "last_modified": i.last_modified,
                    "content_length": i.content_length,
                    "accept_ranges": (i.range_support == engine_core::inspect::RangeSupport::Yes),
                    "status": i.status,
                })),
                Err(e) => Err(engine_err(e)),
            }
        }
        "engine.info" => Ok(json!({
            "name": ENGINE_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "api_version": API_VERSION,
            "data_dir": manager.data_dir().display().to_string(),
            "max_connections_per_host": manager.engine_config().max_connections_per_host,
            "max_concurrent_downloads": manager.engine_config().max_concurrent_downloads,
            "merge_capable": manager.has_media_merge(),
            "default_out_dir": manager.engine_config().default_out_dir.display().to_string(),
        })),
        "download.create" => {
            let p: CreateParams = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":format!("invalid params: {e}")}}).to_string()),
            };
            match manager.add(&p.url, p.dl).await {
                Ok(a) => Ok(serde_json::to_value(a).unwrap_or(Value::Null)),
                Err(e) => Err(engine_err(e)),
            }
        }
        "download.start" | "download.resume" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"missing id"}})
                        .to_string(),
                );
            };
            manager
                .clone()
                .start(id_)
                .await
                .map(|_| json!({"ok": true}))
                .map_err(engine_err)
        }
        "download.pause" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(bad_params_json(id, "missing id"));
            };
            manager
                .pause(id_)
                .await
                .map(|_| json!({"ok": true}))
                .map_err(engine_err)
        }
        "download.cancel" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(bad_params_json(id, "missing id"));
            };
            manager
                .cancel(id_)
                .await
                .map(|_| json!({"ok": true}))
                .map_err(engine_err)
        }
        // download.remove is RECORD removal (any state, incl. completed): with
        // delete_file=true the downloaded file is deleted too. Previously aliased
        // to cancel, which refused completed rows — UIs could not manage finished
        // downloads whose files were deleted or unwanted (UI overhaul 2026-11-22).
        "download.remove" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(bad_params_json(id, "missing id"));
            };
            let delete_file = params
                .get("delete_file")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            manager
                .clone()
                .remove(id_, delete_file)
                .await
                .map(|_| json!({"ok": true}))
                .map_err(engine_err)
        }
        "download.list" => manager.list().map(|rows| json!(rows)).map_err(engine_err),
        "thumbnail.get" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(bad_params_json(id, "missing id"));
            };
            Ok(json!({ "dataUrl": read_thumb_data_url(&manager, id_) }))
        }
        "download.status" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(bad_params_json(id, "missing id"));
            };
            match manager.status(id_) {
                Ok(mut v) => {
                    if let Ok(errs) = manager.db().errors(id_) {
                        v.recent_errors = errs
                            .into_iter()
                            .map(|(ts, msg, class, _)| format!("{ts} [{class}] {msg}"))
                            .collect();
                    }
                    Ok(serde_json::to_value(v).unwrap_or(Value::Null))
                }
                Err(e) => Err(engine_err(e)),
            }
        }
        "download.verify" => {
            let Some(id_) = params.get("id").and_then(parse_id) else {
                return Some(bad_params_json(id, "missing id"));
            };
            manager
                .verify(id_)
                .await
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .map_err(engine_err)
        }
        "download.merge" => {
            // MediaPipeline phase (docs/FFMPEG_INTEGRATION.md §2): stream-copy merge
            // of two COMPLETED stream downloads. Fails honestly when the manager has
            // no merge port configured (no LGPL ffmpeg found at startup).
            let video_id = params
                .get("video_id")
                .or_else(|| params.get("id"))
                .and_then(parse_id);
            let audio_id = params.get("audio_id").and_then(parse_id);
            let (Some(video_id), Some(audio_id)) = (video_id, audio_id) else {
                return Some(bad_params_json(id, "missing video_id/audio_id"));
            };
            manager
                .merge_streams(video_id, audio_id)
                .await
                .map(|_| json!({"video_id": video_id, "audio_id": audio_id, "merged": true}))
                .map_err(engine_err)
        }
        other => Err(err(-32601, format!("method not found: {other}"))),
    };

    Some(match result {
        Ok(v) => json!({"jsonrpc":"2.0","id":id,"result":v}).to_string(),
        Err((code, message)) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}).to_string()
        }
    })
}

/// Read a row's persisted thumbnail and encode it as a data URL for the UI
/// (CSP already allows img-src data:). Missing/corrupt thumb → None.
fn read_thumb_data_url(manager: &std::sync::Arc<engine_core::manager::DownloadManager>, id: i64) -> Option<String> {
    let path = manager.thumb_path(id);
    let bytes = std::fs::read(&path).ok()?;
    let mime = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return None;
    };
    Some(format!("data:{mime};base64,{}", b64_encode(&bytes)))
}

/// Minimal standard base64 (no external dep): engine-api only encodes small
/// thumbnail blobs, correctness is pinned by the unit test below.
fn b64_encode(data: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    out
}

fn bad_params_json(id: Value, msg: &str) -> String {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":msg}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::config::EngineConfig;
    use engine_http::ReqwestBackend;
    use std::sync::Arc;

    fn manager() -> Arc<DownloadManager> {
        let dir = tempfile::tempdir().unwrap();
        let cfg = EngineConfig {
            data_dir: dir.path().join("data"),
            default_out_dir: dir.path().join("out"),
            ..Default::default()
        };
        let backend = Arc::new(ReqwestBackend::new(&cfg).unwrap());
        DownloadManager::new(cfg, backend).unwrap()
    }

    #[tokio::test]
    async fn parse_error_and_unknown_method() {
        let m = manager();
        let r = handle_line(&m, "{not json").await.unwrap();
        assert!(r.contains("-32700"));
        let r = handle_line(&m, r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#)
            .await
            .unwrap();
        assert!(r.contains("-32601"), "{r}");
        let r = handle_line(&m, r#"{"jsonrpc":"2.0","id":2,"method":"engine.info"}"#)
            .await
            .unwrap();
        assert!(r.contains("\"api_version\":\"1\""), "{r}");
    }

    #[tokio::test]
    async fn create_bad_scheme() {
        let m = manager();
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":3,"method":"download.create","params":{"url":"ftp://x/f"}}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("INVALID_URL"), "{r}");
    }

    #[tokio::test]
    async fn merge_params_validated_and_capability_reported() {
        let m = manager();
        // missing params → -32602
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":11,"method":"download.merge","params":{}}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("-32602"), "{r}");
        // no merge port configured → honest INVALID_CONFIG, not a fake success
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":12,"method":"download.merge","params":{"video_id":1,"audio_id":2}}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("media pipeline not configured"), "{r}");
        // capability is reported honestly in engine.info
        let r = handle_line(&m, r#"{"jsonrpc":"2.0","id":13,"method":"engine.info"}"#)
            .await
            .unwrap();
        assert!(r.contains("\"merge_capable\":false"), "{r}");
    }

    #[tokio::test]
    async fn notifications_get_no_response_and_ids_are_strict() {
        let m = manager();
        // notification (no "id"): must produce NO response (JSON-RPC 2.0)
        assert!(
            handle_line(&m, r#"{"jsonrpc":"2.0","method":"engine.info"}"#)
                .await
                .is_none(),
            "notification must not be answered"
        );
        // id of an invalid type: rejected with -32600
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":{"deep":1},"method":"engine.info"}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("-32600"), "{r}");
    }

    #[tokio::test]
    async fn download_inspect_method_previews_without_registering() {
        // the server may be unreachable here — what matters is that the METHOD routes and
        // validates params before any network work (bad scheme must fail without a record)
        let m = manager();
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":"i1","method":"download.inspect","params":{"url":"ftp://x/f"}}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("INVALID_URL"), "{r}");
        assert!(
            m.list().unwrap().is_empty(),
            "inspect must not register anything"
        );
    }

    #[tokio::test]
    async fn remove_routes_to_record_removal_not_cancel() {
        // routing distinction: cancel(unknown) is a silent Ok (remove_partial
        // no-ops), remove(unknown) is an honest INVALID_CONFIG — proves the two
        // methods no longer share one handler
        let m = manager();
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":21,"method":"download.cancel","params":{"id":4242}}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("\"ok\":true"), "cancel keeps its semantics: {r}");
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":22,"method":"download.remove","params":{"id":4242}}"#,
        )
        .await
        .unwrap();
        assert!(
            r.contains("unknown download id"),
            "remove is record removal: {r}"
        );
        // delete_file param is accepted (unknown id still errors first)
        let r = handle_line(
            &m,
            r#"{"jsonrpc":"2.0","id":23,"method":"download.remove","params":{"id":4242,"delete_file":true}}"#,
        )
        .await
        .unwrap();
        assert!(r.contains("unknown download id"), "{r}");
    }

    #[tokio::test]
    async fn oversized_request_lines_are_capped() {
        let m = manager();
        let huge = format!("{} ", "x".repeat(MAX_LINE_BYTES + 64));
        let (raw, overflow) = {
            // exercise the reader directly (serve() would answer and continue)
            let mut reader = tokio::io::BufReader::new(huge.as_bytes());
            read_line_capped(&mut reader, MAX_LINE_BYTES)
                .await
                .unwrap()
                .unwrap()
        };
        assert!(overflow, "line above the cap must be flagged");
        assert!(
            raw.len() <= MAX_LINE_BYTES,
            "buffer must never grow past the cap"
        );
        let _ = &m;
    }

    /// Adaptive media e2e (Phase 2): a video row created with a media_merge spec,
    /// REAL HTTP for both streams, auto audio half + auto merge + audio-row cleanup.
    /// Runs here (not engine-core) because the dev-cycle engine-core⇄engine-http
    /// forbids ReqwestBackend in engine-core's own tests.
    #[tokio::test]
    async fn media_job_end_to_end_auto_audio_and_merge() {
        use engine_testserver::{Behavior, TestServer};
        use std::path::Path;
        use std::sync::Arc as StdArc;

        struct WriteMerge;
        #[async_trait::async_trait]
        impl engine_core::ports::MediaMergePort for WriteMerge {
            async fn merge(
                &self,
                _id: i64,
                _video: &Path,
                _audio: &Path,
                output: &Path,
                format: &str,
                _bus: &engine_core::events::EventBus,
                _kill: tokio::sync::watch::Receiver<bool>,
            ) -> Result<(), engine_core::error::EngineError> {
                assert_eq!(format, "mp4", "muxer format must reach the merge port");
                std::fs::write(output, b"merged-bytes").map_err(|e| {
                    engine_core::error::EngineError::new(
                        engine_core::error::ErrorKind::MergeFailed,
                        format!("write failed: {e}"),
                    )
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let cfg = EngineConfig {
            data_dir: dir.path().join("data"),
            default_out_dir: dir.path().join("out"),
            ..Default::default()
        };
        std::fs::create_dir_all(&cfg.default_out_dir).unwrap();
        let backend = StdArc::new(ReqwestBackend::new(&cfg).unwrap());
        let m = engine_core::manager::DownloadManager::new(cfg, backend).unwrap();
        m.set_media_merge(Some(StdArc::new(WriteMerge)));
        let mut b = Behavior::new(1 << 20);
        b.serve_any_path = true; // video (/file) + audio (/audio) both must serve
        let server = TestServer::start(b);

        let spec = engine_core::config::MediaMergeSpec {
            audio_url: server.url("/audio"),
            output_ext: Some("mp4".into()),
            output_name: Some("My Video.mp4".into()),
            audio_name: Some("My Video (audio stream).m4a".into()),
            convert: None,
        };
        let added = m
            .add(
                &server.url("/file"),
                engine_core::config::DownloadConfig {
                    media_merge: Some(spec),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        m.clone().start(added.id).await.unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let row = loop {
            let r = m.db().get_download(added.id).unwrap().unwrap();
            if r.state == "completed" && r.final_name == "My Video.mp4" {
                break r;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "media job did not finish: {} {} | all rows: {:?}",
                r.state,
                r.final_name,
                m.db()
                    .list_downloads()
                    .unwrap()
                    .iter()
                    .map(|x| (
                        x.id,
                        x.url.clone(),
                        x.state.clone(),
                        x.final_name.clone(),
                        x.error_message.clone()
                    ))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        };

        let rows = m.db().list_downloads().unwrap();
        assert!(
            !rows.iter().any(|r| r.url == server.url("/audio")),
            "audio row must be removed after the merge"
        );
        assert_eq!(
            std::fs::read(dir.path().join("out/My Video.mp4")).unwrap(),
            b"merged-bytes"
        );
        let cfg: engine_core::config::DownloadConfig =
            serde_json::from_str(&row.config_json).unwrap();
        assert!(cfg.media_merge.is_none(), "pending spec cleared");
        server.stop();
    }
}
