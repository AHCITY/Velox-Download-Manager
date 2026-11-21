//! Google Drive URL semantics (publicly shared single files).
//!
//! Real-world flow (verified against current Drive behavior, see docs/ROOT-CAUSE-REPORT):
//! - `https://drive.google.com/file/d/<ID>/view?usp=sharing` is a *web page*, not a file —
//!   downloading it stores the page (that is exactly the "view" failure from the field).
//! - `https://drive.google.com/uc?export=download&id=<ID>` answers with the file directly
//!   (Content-Disposition: attachment) when it is small enough for virus scanning, and
//!   302-redirects to `drive.usercontent.google.com/download` otherwise.
//! - Large files additionally get a "Google Drive can't scan this file for viruses"
//!   interstitial containing a form (`action` + hidden `id/export/confirm/uuid` inputs)
//!   and/or a `Download anyway` anchor; following it with `confirm=t` yields the file.
//! - Private files, folders, quota-exceeded and removed files answer with web pages or
//!   errors — those must surface as honest failures, never as downloads.
//!
//! Only publicly accessible files are supported; authentication-protected content is
//! reported, not bypassed.

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlKind {
    /// Google Drive single-file link; carries the extracted file id.
    GDrive { id: String },
    /// A media PAGE (YouTube watch/shorts/youtu.be) — generic HTTP cannot fetch it.
    Media,
    /// Everything else: plain HTTP semantics.
    Other,
}

/// Classify a user-supplied URL.
pub fn classify_url(url: &str) -> UrlKind {
    let Ok(u) = Url::parse(url) else {
        return UrlKind::Other;
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let path = u.path().to_string();
    if is_youtube(&host, &path) {
        return UrlKind::Media;
    }
    if let Some(id) = drive_file_id(&host, &path, u.query()) {
        return UrlKind::GDrive { id };
    }
    UrlKind::Other
}

const YOUTUBE_HOSTS: [&str; 5] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtu.be",
];

fn is_youtube(host: &str, path: &str) -> bool {
    if host == "youtu.be" {
        return true; // short link to a video page by definition
    }
    YOUTUBE_HOSTS.contains(&host)
        && (path == "/watch"
            || path.starts_with("/watch?")
            || path.starts_with("/shorts/")
            || path.starts_with("/embed/")
            || path.starts_with("/live/"))
}

const DRIVE_HOSTS: [&str; 3] = [
    "drive.google.com",
    "docs.google.com",
    "drive.usercontent.google.com",
];

fn drive_file_id(host: &str, path: &str, query: Option<&str>) -> Option<String> {
    if !DRIVE_HOSTS.contains(&host) {
        return None;
    }
    // folder links are not single-file downloads — do not pretend otherwise
    if path.contains("/folders/") {
        return None;
    }
    // /file/d/<id>/view  (share dialog "view" page; also /file/d/<id>/preview, ...)
    let segments: Vec<&str> = path.split('/').collect();
    if let Some(pos) = segments.iter().position(|s| *s == "d") {
        if let Some(id) = segments.get(pos + 1) {
            if !id.is_empty() && host != "drive.usercontent.google.com" {
                return Some((*id).to_string());
            }
        }
    }
    // uc?export=download&id=..., open?id=..., /download?id=...
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "id" && !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// The canonical download endpoint for a Drive file id (the `/file/d/<id>/view` rewrite).
pub fn direct_download_url(id: &str) -> String {
    format!("https://drive.google.com/uc?export=download&id={id}")
}

/// Minimal HTML entity decoding for attribute values (URLs carry `&amp;`).
fn unescape_entities(v: &str) -> String {
    if !v.contains('&') {
        return v.to_string();
    }
    v.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// Is this page (probably) a Drive download-confirmation interstitial? Deliberately
/// specific: an ordinary error page must NOT be "confirmed" into a download.
fn looks_like_drive_page(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("drive.usercontent.google.com/download")
        || lower.contains("virus scan")
        || (lower.contains("name=\"uuid\"") && lower.contains("confirm"))
        || lower.contains("download anyway")
        || lower.contains("scan this file for viruses")
}

/// Extract the resolved download URL from a Drive confirm interstitial.
///
/// Preference order (matches the live page shapes):
/// 1. the confirm `<form action=...>` with its hidden inputs (`id`, `export=download`,
///    `confirm=t`, `uuid=...`) — the canonical path;
/// 2. a `Download anyway` anchor whose href already carries `confirm=t`.
///
/// Relative actions are resolved against `base`. Returns None when nothing credible
/// (Drive-marker-guarded) is found.
pub fn parse_confirm_page(html: &str, base: &str) -> Option<String> {
    if !looks_like_drive_page(html) {
        return None;
    }
    if let Some(u) = parse_confirm_form(html, base) {
        return Some(u);
    }
    parse_download_anchor(html, base)
}

fn find_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{attr}=");
    let start = lower.find(&needle)? + needle.len();
    // keep original case for the value: slice the same offsets from `tag`
    let rest = &tag[start..];
    let trimmed = rest.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.first() == Some(&b'"') || bytes.first() == Some(&b'\'') {
        let q = bytes[0] as char;
        let end = trimmed[1..].find(q)? + 1;
        Some(unescape_entities(&trimmed[1..end]))
    } else {
        let end = trimmed
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(trimmed.len());
        Some(unescape_entities(&trimmed[..end]))
    }
}

fn parse_confirm_form(html: &str, base: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find("<form") {
        let fpos = from + rel;
        let Some(tag_rel) = lower[fpos..].find('>') else {
            break;
        };
        let tag_end = fpos + tag_rel;
        let form_tag = &html[fpos..=tag_end.min(html.len() - 1)];
        let body_end = lower[fpos..]
            .find("</form")
            .map(|p| fpos + p)
            .unwrap_or(html.len());

        let mut params: Vec<(String, String)> = Vec::new();
        let mut pos = fpos;
        while pos < body_end {
            let Some(irel) = lower[pos..body_end].find("<input") else {
                break;
            };
            let istart = pos + irel;
            let iend = lower[istart..body_end]
                .find('>')
                .map(|p| istart + p)
                .unwrap_or(body_end.saturating_sub(1));
            let input_tag = &html[istart..=iend.min(html.len() - 1)];
            if let (Some(n), Some(v)) =
                (find_attr(input_tag, "name"), find_attr(input_tag, "value"))
            {
                params.push((n.to_ascii_lowercase(), v));
            }
            pos = iend + 1;
        }

        // guard: a Drive confirm form carries the file id (as input or in the action)
        let action = find_attr(form_tag, "action").unwrap_or_default();
        let has_id =
            params.iter().any(|(k, _)| k == "id") || action.to_ascii_lowercase().contains("id=");
        if has_id && !params.is_empty() {
            let mut url = Url::parse(if action.is_empty() { base } else { &action })
                .or_else(|_| Url::parse(base).and_then(|b| b.join(&action)))
                .ok()?;
            {
                let mut qp = url.query_pairs_mut();
                for (k, v) in &params {
                    // append (not replace): duplicates are harmless for the Drive
                    // endpoint — the last pair wins — and the serializer has no removal
                    qp.append_pair(k, v);
                }
                // the documented bypass: the confirm parameter is what Drive's own
                // "Download anyway" submits; add it when the form omitted it
                if !params.iter().any(|(k, _)| k == "confirm") {
                    qp.append_pair("confirm", "t");
                }
            }
            return Some(url.to_string());
        }

        from = tag_end + 1;
    }
    None
}

fn parse_download_anchor(html: &str, base: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(arel) = lower[from..].find("<a ") {
        let astart = from + arel;
        let aend = lower[astart..]
            .find('>')
            .map(|p| astart + p)
            .unwrap_or(astart);
        let tag = &html[astart..=aend.min(html.len() - 1)];
        if let Some(href) = find_attr(tag, "href") {
            let hl = href.to_ascii_lowercase();
            if hl.contains("confirm=t") || hl.contains("confirm%3dt") {
                if let Ok(u) =
                    Url::parse(&href).or_else(|_| Url::parse(base).and_then(|b| b.join(&href)))
                {
                    return Some(u.to_string());
                }
            }
        }
        from = aend + 1;
    }
    None
}

/// Honest, specific diagnosis of why a Drive response was a page and not a file.
pub fn page_reason(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if lower.contains("accounts.google.com") || lower.contains("sign in") {
        "the file requires sign-in — it is not publicly accessible, and Velox does not \
         access private files."
            .to_string()
    } else if lower.contains("too many users have viewed")
        || lower.contains("quota")
        || lower.contains("rate limit")
    {
        "the download quota for this file has been exceeded; try again later.".to_string()
    } else if lower.contains("doesn't exist") || lower.contains("unavailable") {
        "the file appears to be removed, unavailable, or the link is wrong.".to_string()
    } else if lower.contains("virus scan")
        || lower.contains("download anyway")
        || lower.contains("scan this file for viruses")
    {
        "a confirmation page was detected but its download form could not be parsed \
         (the page format may have changed)."
            .to_string()
    } else {
        "the link may be private, a folder, removed, or not a file link. Velox downloads \
         only publicly shared single files from Drive."
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIRM_PAGE: &str = r#"<!DOCTYPE html><html><head><title>Google Drive - Virus scan warning</title></head>
    <body><div class="uc-main"><p>Google Drive can't scan this file for viruses.</p>
    <form id="download-form" action="https://drive.usercontent.google.com/download" method="get">
      <input type="hidden" name="id" value="1aIvApuzF7NlNX4-lOpalfnXspzR3uZNg13">
      <input type="hidden" name="export" value="download">
      <input type="hidden" name="confirm" value="t">
      <input type="hidden" name="uuid" value="0aa1b2c3-d4e5-6789-abcd-ef0123456789">
      <input type="submit" id="uc-download-link" value="Download anyway">
    </form></div></body></html>"#;

    #[test]
    fn classify_recognizes_drive_and_media_urls() {
        assert_eq!(
            classify_url("https://drive.google.com/file/d/1aIvApuzF7NlNX4-lOpalfnXspzR3uZNg13/view?usp=sharing"),
            UrlKind::GDrive { id: "1aIvApuzF7NlNX4-lOpalfnXspzR3uZNg13".into() }
        );
        assert_eq!(
            classify_url("https://drive.google.com/uc?export=download&id=abc123"),
            UrlKind::GDrive {
                id: "abc123".into()
            }
        );
        assert_eq!(
            classify_url("https://drive.usercontent.google.com/download?id=abc123&export=download"),
            UrlKind::GDrive {
                id: "abc123".into()
            }
        );
        assert_eq!(
            classify_url("https://www.youtube.com/watch?v=XdNjEct5mqc"),
            UrlKind::Media
        );
        assert_eq!(classify_url("https://youtu.be/XdNjEct5mqc"), UrlKind::Media);
        assert_eq!(
            classify_url("https://www.youtube.com/shorts/abcXYZ123"),
            UrlKind::Media
        );
        assert_eq!(
            classify_url("https://drive.google.com/drive/folders/1a2b3c"),
            UrlKind::Other,
            "folders are not single files"
        );
        assert_eq!(classify_url("https://example.com/file.zip"), UrlKind::Other);
    }

    #[test]
    fn confirm_form_parses_action_and_inputs() {
        let url = parse_confirm_page(
            CONFIRM_PAGE,
            "https://drive.google.com/uc?export=download&id=x",
        )
        .unwrap();
        assert!(url.starts_with("https://drive.usercontent.google.com/download?"));
        for part in [
            "id=1aIvApuzF7NlNX4-lOpalfnXspzR3uZNg13",
            "export=download",
            "confirm=t",
            "uuid=0aa1b2c3-d4e5-6789-abcd-ef0123456789",
        ] {
            assert!(url.contains(part), "missing {part} in {url}");
        }
    }

    #[test]
    fn relative_action_resolves_against_base() {
        let page = r#"<html><body>Google Drive can't scan this file for viruses
        <form action="/download" method="get"><input type="hidden" name="id" value="abc">
        <input type="hidden" name="confirm" value="t"></form></body></html>"#;
        let url = parse_confirm_page(page, "https://drive.usercontent.google.com/uc?id=x").unwrap();
        assert!(
            url.starts_with("https://drive.usercontent.google.com/download?"),
            "{url}"
        );
        assert!(url.contains("id=abc") && url.contains("confirm=t"));
    }

    #[test]
    fn download_anyway_anchor_is_fallback() {
        let page = r#"<html><body><p>Google Drive can't scan this file for viruses.</p>
        <a href="https://drive.usercontent.google.com/download?id=abc&amp;export=download&amp;confirm=t&amp;uuid=u1">Download anyway</a></body></html>"#;
        let url = parse_confirm_page(page, "https://drive.google.com/uc?id=x").unwrap();
        assert!(url.contains("id=abc") && url.contains("confirm=t") && !url.contains("&amp;"));
    }

    #[test]
    fn ordinary_pages_are_not_confirmed() {
        assert!(
            parse_confirm_page("<html><body><h1>Not found</h1></body></html>", "https://x/")
                .is_none()
        );
        assert!(parse_confirm_page(
            "<html><body><form action=\"/login\"><input name=\"user\"></form></body></html>",
            "https://x/"
        )
        .is_none());
    }

    #[test]
    fn page_reason_is_specific() {
        assert!(
            page_reason("<a href='https://accounts.google.com/ServiceLogin'>Sign in</a>")
                .contains("sign-in")
        );
        assert!(
            page_reason("too many users have viewed or downloaded this file recently")
                .contains("quota")
        );
        assert!(
            page_reason("Google Drive can't scan this file for viruses").contains("confirmation")
        );
    }
}
