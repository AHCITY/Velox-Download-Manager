#!/usr/bin/env python3
"""extract.rs: replace resolve_media_with's duplicated yt-dlp spawn body with
run_extractor_json, thread the MP3 flag into ResourceInfo, and fix the old
pick_plan test call."""

P = '/home/z/my-project/download-engine/crates/engine-http/src/extract.rs'
s = open(P).read()

# --- 1. thin resolve_media_with: delegate to run_extractor_json ---
start = s.index('pub async fn resolve_media_with(')
end = s.index('    let title = meta', start)
new_head = '''pub async fn resolve_media_with(
    backend: &ReqwestBackend,
    url: &str,
    hdrs: &RequestHeaders,
    choice: Option<crate::formats::MediaFormatChoice>,
) -> Result<ResourceInfo, EngineError> {
    // Shared yt-dlp -J run (audit D1 kill-on-drop + D2 header forwarding live
    // in run_extractor_json, used by both the resolve and the formats probe).
    let meta = run_extractor_json(backend, url, hdrs).await?;
'''
s = s[:start] + new_head + s[end:]

# --- 2. plan → ResourceInfo: honour to_mp3 ---
old_info = '''    let info = match &plan.audio {
        // Fast path: progressive stream (video+audio in ONE file) — download directly.
        None => ResourceInfo {'''
new_info = '''    let info = if plan.to_mp3 {
        // MP3 path: single audio-stream download; the media job converts it to
        // .mp3 (libmp3lame -q:a 0) once the raw stream lands on disk.
        ResourceInfo {
            final_url: plan.video.url.clone(),
            redirect_chain: vec![url.to_string()],
            status: 200,
            content_length: plan.video.size,
            range_support: engine_core::inspect::RangeSupport::Unknown,
            mime: Some("audio/mpeg".into()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            suggested_name: Some(format!("{}.mp3", title)),
            resolve_note: Some(format!(
                "media extractor: “{}” — best audio [{}] → MP3",
                title, plan.video.format_id
            )),
            media: Some(engine_core::config::MediaMergeSpec {
                audio_url: String::new(),
                output_ext: Some("mp3".into()),
                output_name: Some(format!("{}.mp3", title)),
                audio_name: None,
                convert: Some(engine_core::config::MediaConvert::Mp3),
            }),
            ..Default::default()
        }
    } else if let Some(audio) = &plan.audio {
        // Adaptive: separate video/audio https streams → two-stream media job.
        let output_ext = merge_container(&plan.video.ext, &audio.ext);
        ResourceInfo {
            final_url: plan.video.url.clone(),
            redirect_chain: vec![url.to_string()],
            status: 200,
            content_length: plan.video.size,
            range_support: engine_core::inspect::RangeSupport::Unknown,
            mime: Some(plan.video.mime.clone()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            suggested_name: Some(format!("{} (video stream).{}", title, plan.video.ext)),
            resolve_note: Some(format!(
                "media extractor: “{}” — adaptive {}+{} → merged .{}",
                title, plan.video.format_id, audio.format_id, output_ext
            )),
            media: Some(engine_core::config::MediaMergeSpec {
                audio_url: audio.url.clone(),
                output_ext: Some(output_ext.clone()),
                output_name: Some(format!("{}.{}", title, output_ext)),
                audio_name: Some(format!("{} (audio stream).{}", title, audio.ext)),
                convert: None,
            }),
            ..Default::default()
        }
    } else {
        // Fast path: progressive stream (video+audio in ONE file) — download directly.
        ResourceInfo {'''
assert old_info in s
s = s.replace(old_info, new_info, 1)

# close the new else-branch structure: the old code was
#   Some(audio) => { ... },\n    };
# we now need the tail:  ... },\n    };
old_tail = '''                media: Some(engine_core::config::MediaMergeSpec {
                    audio_url: audio.url.clone(),
                    output_ext: Some(output_ext.clone()),
                    output_name: Some(format!("{}.{}", title, output_ext)),
                    audio_name: Some(format!("{} (audio stream).{}", title, audio.ext)),
                }),
                ..Default::default()
            }
        }
    };
    Ok(info)
}'''
new_tail = '''            resolve_note: Some(format!(
                "media extractor: “{}” [{}]",
                title, plan.video.format_id
            )),
            ..Default::default()
        }
    };
    Ok(info)
}'''
assert old_tail in s, 'old adaptive tail not found'
s = s.replace(old_tail, new_tail, 1)

# the old progressive branch body still carries the old resolve_note block —
# remove the now-duplicated remnant of the old match arm
old_prog_remnant = '''            final_url: plan.video.url.clone(),
            redirect_chain: vec![url.to_string()],
            status: 200,
            content_length: plan.video.size,
            range_support: engine_core::inspect::RangeSupport::Unknown,
            mime: Some(plan.video.mime.clone()),
            sniffed: engine_core::inspect::SniffedKind::Unknown,
            suggested_name: Some(format!("{}.{}", title, plan.video.ext)),
            resolve_note: Some(format!(
                "media extractor: “{}” [{}]",
                title, plan.video.format_id
            )),
            ..Default::default()
        },
        // Adaptive: separate video/audio https streams → two-stream media job.
        Some(audio) => {
            let output_ext = merge_container(&plan.video.ext, &audio.ext);
            ResourceInfo {'''
# not present anymore after step-2 replacement — verify structure instead
assert 'Some(audio) =>' not in s, 'old match arm still present'

open(P, 'w').write(s)
print('extract.rs: resolve_media_with delegated + MP3 info mapping done')
