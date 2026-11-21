//! Filename resolution: Content-Disposition (RFC 6266 + RFC 8187), URL paths, sanitization
//! (RFC 6266 §4.3/App C rules; Windows reserved names; traversal impossible by construction).

use crate::inspect::ResourceInfo;

/// Reduce any candidate to a safe, single path component. Preserves Unicode.
pub fn sanitize_filename(raw: &str) -> String {
    // 1) keep only the final path segment for both separators
    let seg = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    // 2) strip control chars; replace separators/Windows-unsafe characters
    let mut out = String::with_capacity(seg.len());
    for c in seg.chars() {
        match c {
            '\0'..='\x1f' | '\x7f' => {}
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            _ => out.push(c),
        }
    }
    let mut s = out.trim().to_string();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s == "." || s == ".." {
        s = "_".to_string();
    }
    if s.starts_with('-') {
        s.insert(0, '_');
    }
    // 3) Windows reserved device names (case-insensitive, with or without extension)
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        s.insert(0, '_');
    }
    if s.is_empty() {
        return "download".to_string();
    }
    // 4) cap length on a char boundary (~240 bytes keeps full path under Windows limits)
    if s.len() > 240 {
        let mut end = 240;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        while s.ends_with('.') || s.ends_with(' ') {
            s.pop();
        }
    }
    s
}

/// Split a header value on ';' respecting double-quoted strings.
fn split_params(v: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in v.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => {
                cur.push(c);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ';' if !in_quotes => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        v.to_string()
    }
}

/// RFC 8187 ext-value: charset'language'percent-encoded-value.
fn decode_ext_value(v: &str) -> String {
    let v = v.trim();
    let mut it = v.splitn(3, '\'');
    let charset = it.next().unwrap_or("").trim().to_ascii_lowercase();
    let _lang = it.next().unwrap_or("");
    let value = it.next().unwrap_or("");
    let decoded = percent_encoding::percent_decode_str(value).collect::<Vec<u8>>();
    match charset.as_str() {
        "iso-8859-1" | "latin1" => decoded.iter().map(|&b| b as char).collect(),
        // utf-8 is the mandated default; unknown charsets: best-effort UTF-8
        _ => String::from_utf8_lossy(&decoded).into_owned(),
    }
}

/// Extract a filename from a Content-Disposition header value.
/// `filename*` wins over `filename` (RFC 6266 §4.3) — but only with a non-empty value;
/// `filename*=""` falls back to `filename` instead of wiping it. Sloppy real-world
/// forms tolerated.
pub fn filename_from_disposition(v: &str) -> Option<String> {
    let mut star: Option<String> = None;
    let mut plain: Option<String> = None;
    for part in split_params(v) {
        let part = part.trim();
        let Some((k, val)) = part.split_once('=') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        if k == "filename*" {
            star = Some(decode_ext_value(val));
        } else if k == "filename" {
            plain = Some(unquote(val));
        }
    }
    let raw = star.filter(|s| !s.trim().is_empty()).or(plain)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(sanitize_filename(raw))
}

/// Last non-empty, percent-decoded URL path segment.
pub fn filename_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let seg = parsed.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_encoding::percent_decode_str(seg).decode_utf8_lossy();
    if decoded.trim().is_empty() {
        return None;
    }
    Some(sanitize_filename(&decoded))
}

/// Path segments that are routes, not file names (`/watch?v=…`, `…/file/d/<id>/view`,
/// `/uc?export=download`). A URL name built from one of these is how "watch" and "view"
/// ended up as download names (P0 root cause #2).
const ROUTE_SEGMENTS: [&str; 18] = [
    "watch", "view", "file", "files", "download", "get", "open", "uc", "d", "index", "drive",
    "folders", "shorts", "embed", "media", "status", "detail", "details",
];

/// Extensions that name a *generator script or page*, not a file payload.
const PAGE_EXTENSIONS: [&str; 12] = [
    "php", "asp", "aspx", "jsp", "cgi", "do", "action", "ashx", "pl", "py", "rb", "shtml",
];

/// A segment that plausibly names a file, as opposed to a route. Rules:
/// - a real extension (1-8 alnum chars) that is not a page-script extension → strong;
/// - without an extension the segment is only trusted when the URL carries no query
///   string (e.g. `/files/README`), the name is not a known route word, and it does not
///   look like an opaque resource id (Drive/YouTube ids: long, high-entropy, no dot).
fn strict_segment(seg: &str, url_has_query: bool) -> Option<&str> {
    let seg = seg.trim();
    if seg.is_empty() || seg.len() > 128 {
        return None;
    }
    let lower = seg.to_ascii_lowercase();
    if ROUTE_SEGMENTS.contains(&lower.as_str()) {
        return None;
    }
    match seg.rsplit_once('.') {
        Some((stem, ext)) => {
            if stem.is_empty() {
                return None; // ".gitignore"-style dotfiles: keep only via MIME/disposition
            }
            let ext_ok = !ext.is_empty()
                && ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && !PAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str());
            ext_ok.then_some(seg)
        }
        None => {
            // no extension: reject opaque ids (hex strings, long base64url tokens) and
            // anything on a URL with a query string — both are route ids, not names
            if url_has_query || seg.len() >= 16 {
                return None;
            }
            if seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && seg.len() >= 12
            {
                return None; // 12+ alnum/-/_ chars with no separator: id-shaped
            }
            Some(seg)
        }
    }
}

/// Last URL path segment ONLY when it plausibly names a file (`/videos/clip.mp4` yes,
/// `/watch?v=…` no, `…/file/d/1a2b…/view` no). Percent-decoding applies.
pub fn filename_from_url_strict(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let has_query = parsed.query().map(|q| !q.is_empty()).unwrap_or(false);
    let seg = parsed.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_encoding::percent_decode_str(seg).decode_utf8_lossy();
    let seg = strict_segment(&decoded, has_query)?;
    let name = sanitize_filename(seg);
    (name != "download").then_some(name)
}

/// MIME types that reliably identify a file payload, mapped to their canonical extension.
/// `application/octet-stream` maps to None on purpose: "unknown binary" is not evidence
/// for an extension, and inventing `.bin` is exactly the blind-rename failure mode.
pub fn ext_for_mime(mime: &str) -> Option<&'static str> {
    let base = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Some(match base.as_str() {
        "application/zip" => "zip",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-compressed-tar" => "tar.gz",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-xz" => "xz",
        "application/x-bzip2" => "bz2",
        "application/zstd" => "zst",
        "application/pdf" => "pdf",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/iso-disk" | "application/x-iso9660-image" => "iso",
        "application/epub+zip" => "epub",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "application/vnd.android.package-archive" => "apk",
        "application/x-msdownload"
        | "application/x-msdownload-format"
        | "application/x-ms-installer" => "exe",
        "application/x-msi" => "msi",
        "application/x-apple-diskimage" => "dmg",
        "application/x-debian-package" | "application/vnd.debian.binary-package" => "deb",
        "application/x-rpm" => "rpm",
        "application/wasm" => "wasm",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "text/markdown" => "md",
        "text/calendar" => "ics",
        "text/tab-separated-values" => "tsv",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/x-icon" => "ico",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-msvideo" => "avi",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mp4" => "m4a",
        "audio/aac" => "aac",
        "font/ttf" | "font/sfnt" => "ttf",
        "font/otf" => "otf",
        "font/woff" => "woff",
        "font/woff2" => "woff2",
        _ => return None,
    })
}

/// Does `name` already carry a usable file extension?
fn has_file_extension(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && !PAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        }
        None => false,
    }
}

/// Keep a resolved name but give it the MIME-derived extension when it has none (or only
/// a page-script one): `video` + video/mp4 → `video.mp4`; `movie.mp4` stays untouched.
/// Unreliable MIME (octet-stream, unknown) never invents an extension.
pub fn reconcile_extension(name: &str, mime: Option<&str>) -> String {
    if has_file_extension(name) {
        return name.to_string();
    }
    let ext = mime.and_then(ext_for_mime);
    match ext {
        Some(e) if !name.is_empty() => format!("{name}.{e}"),
        _ => name.to_string(),
    }
}

/// Resolution order (user-mandated priority):
/// 1. explicit user-provided filename;
/// 2. valid Content-Disposition filename;
/// 3. final resolved resource name (extractor/Drive-proposed name, then the strict URL
///    path name — route segments like `watch`/`view` are never used bare; a weak stem
///    gains the MIME-derived extension instead: `/watch` + video/mp4 → `watch.mp4`);
/// 4. reliable MIME-based extension → `download-<id>.<ext>`;
/// 5. safe fallback `download-<id>` (no invented `.bin`/`.html`).
pub fn choose_filename(info: &ResourceInfo, explicit: Option<&str>, id: i64) -> String {
    if let Some(f) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return sanitize_filename(f);
    }
    if let Some(cd) = &info.content_disposition {
        if let Some(f) = filename_from_disposition(cd) {
            return f;
        }
    }
    if let Some(s) = info
        .suggested_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return reconcile_extension(&sanitize_filename(s), info.mime.as_deref());
    }
    if let Some(f) = filename_from_url_strict(&info.final_url) {
        return reconcile_extension(&f, info.mime.as_deref());
    }
    // weak URL stem (route word, id, extension-less): usable only WITH a reliable MIME
    // extension — `watch` + video/mp4 → `watch.mp4`; bare route words never stand alone
    if let Some(stem) = filename_from_url(&info.final_url) {
        if let Some(e) = info.mime.as_deref().and_then(ext_for_mime) {
            let candidate = sanitize_filename(&format!("{stem}.{e}"));
            if has_file_extension(&candidate) {
                return candidate;
            }
        }
    }
    if let Some(ext) = info.mime.as_deref().and_then(ext_for_mime) {
        return format!("download-{id}.{ext}");
    }
    format!("download-{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_blocks_traversal_and_devices() {
        assert_eq!(sanitize_filename("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename("/etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\temp\\x"), "x");
        assert_eq!(
            sanitize_filename("a/b:c"),
            "b_c",
            "only the final segment survives"
        );
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("com1.txt"), "_com1.txt");
        assert_eq!(
            sanitize_filename(".."),
            "download",
            "dot-only names collapse to the fallback"
        );
        assert_eq!(sanitize_filename("   "), "download");
        assert_eq!(sanitize_filename("name. . "), "name");
        assert_eq!(sanitize_filename("日本語.zip"), "日本語.zip");
        assert_eq!(sanitize_filename("-flag"), "_-flag");
        let long = "x".repeat(500);
        assert_eq!(sanitize_filename(&long).len(), 240);
    }

    #[test]
    fn disposition_parsing() {
        // RFC 6266 §4.3: filename* wins
        let f = filename_from_disposition(
            "attachment; filename=\"fallback.bin\"; filename*=UTF-8''na%C3%AFve%20file.txt",
        )
        .unwrap();
        assert_eq!(f, "naïve file.txt");
        let f = filename_from_disposition("attachment; filename=\"plain name.zip\"").unwrap();
        assert_eq!(f, "plain name.zip");
        // sloppy: unquoted with spaces
        let f = filename_from_disposition("attachment; filename=unquoted file.bin").unwrap();
        assert_eq!(f, "unquoted file.bin");
        // quoted-string escapes
        let f = filename_from_disposition("attachment; filename=\"we\\\"ird.zip\"").unwrap();
        assert_eq!(
            f, "we_ird.zip",
            "quote is unsafe on Windows → replaced by '_'"
        );
        // latin-1 charset
        let f = filename_from_disposition("attachment; filename*=ISO-8859-1''caf%E9.mp3").unwrap();
        assert_eq!(f, "café.mp3");
        // traversal attempt inside disposition is sanitized
        let f = filename_from_disposition("attachment; filename=\"..\\..\\win.ini\"").unwrap();
        assert_eq!(f, "win.ini");
        assert!(filename_from_disposition("attachment").is_none());
    }

    #[test]
    fn url_names_and_mime() {
        assert_eq!(
            filename_from_url("https://x/y/a%20b.tar.gz").unwrap(),
            "a b.tar.gz"
        );
        assert!(filename_from_url("https://x/").is_none());
        assert_eq!(ext_for_mime("application/zip; charset=binary"), Some("zip"));
        assert_eq!(ext_for_mime("weird/thing"), None);
        assert_eq!(
            ext_for_mime("application/octet-stream"),
            None,
            "unknown binary never invents .bin"
        );
    }

    #[test]
    fn strict_url_names_reject_routes_and_ids() {
        assert_eq!(
            filename_from_url_strict("https://cdn.example/videos/clip.mp4").unwrap(),
            "clip.mp4"
        );
        assert_eq!(
            filename_from_url_strict("https://x/y/a%20b.tar.gz").unwrap(),
            "a b.tar.gz"
        );
        // the P0 failure modes: route words as names
        assert!(filename_from_url_strict("https://www.youtube.com/watch?v=XdNjEct5mqc").is_none());
        assert!(filename_from_url_strict(
            "https://drive.google.com/file/d/1aIvApuzF7NlNX4-lOpalfnXspzR3uZNg13/view?usp=sharing"
        )
        .is_none());
        assert!(
            filename_from_url_strict("https://drive.google.com/uc?export=download&id=x").is_none()
        );
        // opaque ids without extension
        assert!(
            filename_from_url_strict("https://cdn.example/api/files/1aIvApuzF7NlNX4").is_none()
        );
        assert!(filename_from_url_strict("https://x/y/deadbeefdeadbeef").is_none());
        // extension-less plain name without query: allowed
        assert_eq!(
            filename_from_url_strict("https://x/y/README").unwrap(),
            "README"
        );
        // page-script extensions are not file names
        assert!(filename_from_url_strict("https://x/download.php").is_none());
    }

    #[test]
    fn disposition_empty_star_falls_back_to_plain() {
        let f = filename_from_disposition("attachment; filename=\"video.mp4\"; filename*=\"\"")
            .unwrap();
        assert_eq!(f, "video.mp4");
    }

    #[test]
    fn reconcile_appends_reliable_ext_only() {
        assert_eq!(reconcile_extension("video", Some("video/mp4")), "video.mp4");
        assert_eq!(
            reconcile_extension("movie.mp4", Some("video/mp4")),
            "movie.mp4"
        );
        assert_eq!(
            reconcile_extension("page", Some("application/octet-stream")),
            "page",
            "octet-stream must not invent .bin"
        );
        assert_eq!(
            reconcile_extension("index.php", Some("application/pdf")),
            "index.php.pdf"
        );
        assert_eq!(reconcile_extension("archive", None), "archive");
    }

    #[test]
    fn choose_order() {
        let mut info = ResourceInfo {
            final_url: "https://cdn.example.net/files/real-name-1.2.3.deb".into(),
            content_disposition: Some(
                "attachment; filename*=UTF-8''%E3%83%95%E3%82%A1%E3%82%A4%E3%83%AB.deb".into(),
            ),
            mime: Some("application/vnd.debian.binary-package".into()),
            ..Default::default()
        };
        assert_eq!(choose_filename(&info, None, 7), "ファイル.deb");
        info.content_disposition = None;
        assert_eq!(choose_filename(&info, None, 7), "real-name-1.2.3.deb");
        info.final_url = "https://x/".into();
        assert_eq!(
            choose_filename(&info, None, 7),
            "download-7.deb",
            "reliable MIME wins the fallback extension"
        );
        assert_eq!(
            choose_filename(&info, Some("custom name.bin"), 7),
            "custom name.bin"
        );
    }

    #[test]
    fn choose_weak_stem_gets_mime_extension() {
        // the /watch path is a route; with a real mp4 payload the name is watch.mp4 —
        // never the bare route word (P0 root cause #2)
        let info = ResourceInfo {
            final_url: "https://example.com/watch".into(),
            mime: Some("video/mp4".into()),
            ..Default::default()
        };
        assert_eq!(choose_filename(&info, None, 3), "watch.mp4");
        // no MIME either → safe fallback, no invented extension
        let bare = ResourceInfo {
            final_url: "https://example.com/1aIvApuzF7NlNX4".into(),
            mime: Some("application/octet-stream".into()),
            ..Default::default()
        };
        assert_eq!(choose_filename(&bare, None, 4), "download-4");
    }

    #[test]
    fn choose_suggested_name_slot() {
        let info = ResourceInfo {
            final_url: "https://r4---sn.example.googlevideo.com/videoplayback?id=o-x".into(),
            mime: Some("video/mp4".into()),
            suggested_name: Some("My Video 1080p.mp4".into()),
            ..Default::default()
        };
        assert_eq!(choose_filename(&info, None, 9), "My Video 1080p.mp4");
    }
}
