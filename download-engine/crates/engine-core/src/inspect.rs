//! Remote resource metadata discovered by inspection (docs/PROTOCOL_COMPATIBILITY.md §1).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RangeSupport {
    #[default]
    Unknown,
    /// Confirmed by an actual ranged GET returning 206.
    Yes,
    /// Confirmed by a ranged GET being answered with 200 (server ignored/rejected ranges).
    No,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceInfo {
    pub final_url: String,
    pub redirect_chain: Vec<String>,
    pub status: u16,
    /// Complete representation length (from Content-Length or Content-Range total).
    pub content_length: Option<u64>,
    pub range_support: RangeSupport,
    /// Raw (quoted) ETag value.
    pub etag: Option<String>,
    pub etag_is_weak: bool,
    pub last_modified: Option<String>,
    pub content_disposition: Option<String>,
    pub mime: Option<String>,
    pub content_encoding: Option<String>,
    pub www_authenticate: Option<String>,
    /// Retry-After from the response, when present (RFC 9110 §10.2.3).
    pub retry_after: Option<std::time::Duration>,
    /// Classification of the first body bytes (identity decoding assumed). The probe reads a
    /// bounded prefix so a web page can never silently pass as a file download (P0 fix).
    pub sniffed: SniffedKind,
    /// Name proposed by the URL-semantics layer (media extractor title, Drive metadata).
    /// Feeds the "final resource name" slot in naming::choose_filename.
    pub suggested_name: Option<String>,
    /// Human-readable provenance note from URL resolution (e.g. "Google Drive: confirmed").
    pub resolve_note: Option<String>,
    /// Poster/thumbnail URL reported by the media extractor (yt-dlp `thumbnail`).
    /// Works for EVERY supported site, not just YouTube; the manager persists the
    /// image to the thumbs dir so the UI keeps it across restarts.
    pub thumbnail_url: Option<String>,
    /// Adaptive media plan from the extractor (yt-dlp): the video half is `final_url`
    /// itself; `media.audio_url` is the second stream and `media.output_name` the final
    /// merged artifact. The manager turns this into a two-stream media job.
    pub media: Option<crate::config::MediaMergeSpec>,
}

/// What the first bytes of a body look like. Deliberately conservative: the engine only
/// asserts confident HTML; everything else stays Unknown rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SniffedKind {
    #[default]
    Unknown,
    /// Confident: a hypertext document (watch pages, Drive interstitials, error pages).
    Html,
    /// Confident: binary payload (NUL bytes or high share of non-ASCII bytes).
    Binary,
}

/// Classify a body prefix. A body is HTML when, after skipping a BOM and whitespace, it
/// starts with a well-known HTML tag/declaration. NUL bytes anywhere in the prefix, or a
/// high share of non-ASCII bytes, mark binary content (this keeps PDFs, ZIPs and media
/// from ever being flagged, even when their header string contains "<" by coincidence).
pub fn sniff_kind(bytes: &[u8]) -> SniffedKind {
    let body = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => rest,
        None => bytes,
    };
    let trimmed: &[u8] = {
        let start = body
            .iter()
            .take(64)
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(0);
        &body[start.min(body.len())..]
    };
    const MARKERS: [&[u8]; 8] = [
        b"<html",
        b"<!doctype",
        b"<head",
        b"<body",
        b"<script",
        b"<meta",
        b"<title",
        b"<div",
    ];
    let lower = trimmed
        .iter()
        .take(64)
        .map(|b| b.to_ascii_lowercase())
        .collect::<Vec<u8>>();
    if MARKERS.iter().any(|m| lower.starts_with(m)) {
        return SniffedKind::Html;
    }
    // marker prefix cut short by the read window: "<html" was truncated — check tag-open form
    if lower.first() == Some(&b'<') && lower.len() >= 2 {
        let tag = lower[1..]
            .iter()
            .take(9)
            .copied()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'!' || *b == b'/')
            .collect::<Vec<u8>>();
        let tag = String::from_utf8_lossy(&tag).to_ascii_lowercase();
        if matches!(
            tag.as_str(),
            "html" | "!doctype" | "head" | "body" | "script" | "meta" | "title" | "div"
        ) {
            return SniffedKind::Html;
        }
    }
    if trimmed.contains(&0u8) {
        return SniffedKind::Binary;
    }
    let sample = &trimmed[..trimmed.len().min(512)];
    if sample.len() >= 64 {
        let non_ascii = sample.iter().filter(|b| !b.is_ascii()).count();
        if non_ascii * 10 > sample.len() * 3 {
            return SniffedKind::Binary;
        }
    }
    SniffedKind::Unknown
}

/// Retry-After (RFC 9110 §10.2.3): delta-seconds or IMF-fixdate HTTP-date. The obsolete
/// date forms (RFC 850, asctime — which senders must not generate) deliberately parse to
/// None → exponential backoff; documented deviation from the recipient MUST, safe direction.
/// Values above the caller's max_delay are clamped by RetryPolicy::delay_for (audit A5).
pub fn parse_retry_after(v: &str) -> Option<std::time::Duration> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    parse_retry_after_at(v, now)
}

/// Testable core: `now_epoch` is seconds since the Unix epoch (UTC).
pub fn parse_retry_after_at(v: &str, now_epoch: u64) -> Option<std::time::Duration> {
    let s = v.trim();
    if let Ok(secs) = s.parse::<u64>() {
        return Some(std::time::Duration::from_secs(secs));
    }
    // strip the weekday prefix ("Wed, ") before parsing the date part
    let date_part = s.split_once(',').map(|(_, r)| r.trim()).unwrap_or(s);
    let date = parse_imf_fixdate(date_part)?;
    // date in the past → retry immediately (standard client practice)
    Some(std::time::Duration::from_secs(
        date.saturating_sub(now_epoch),
    ))
}

/// IMF-fixdate only (RFC 9110 §5.6.7): `Sun, 06 Nov 1994 08:49:37 GMT`.
fn parse_imf_fixdate(s: &str) -> Option<u64> {
    let mut it = s.split_whitespace();
    let day: u32 = it.next()?.parse().ok()?;
    let mon = month_num(it.next()?)?;
    let year: i64 = it.next()?.parse().ok()?;
    let (h, mi, sec) = parse_hms(it.next()?)?;
    if !it.next()?.eq_ignore_ascii_case("GMT") {
        return None;
    }
    if it.next().is_some() {
        return None;
    }
    if !(1..=31).contains(&day) || !(0..=23).contains(&h) || sec > 60 {
        return None;
    }
    Some(
        (days_from_civil(year, mon, day) * 86_400 + h as i64 * 3_600 + mi as i64 * 60 + sec as i64)
            as u64,
    )
}

fn month_num(m: &str) -> Option<u32> {
    let l = m.to_ascii_lowercase();
    match l.as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

fn parse_hms(s: &str) -> Option<(u32, u32, u32)> {
    let (h, rest) = s.split_once(':')?;
    let (m, sec) = rest.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?, sec.parse().ok()?))
}

/// Days since 1970-01-01 from a proleptic Gregorian civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m as i64) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl ResourceInfo {
    /// RFC 9110 §13.1.5/§8.8.3: only strong ETags may drive If-Range.
    pub fn strong_etag(&self) -> Option<&str> {
        if self.etag_is_weak {
            None
        } else {
            self.etag.as_deref()
        }
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A 416 answered with `Content-Range: bytes */N` is a *usable* probe outcome: it carries the
/// current complete length (RFC 9110 §15.5.17). Zero-length resources always answer this way.
pub fn probe_usable(info: &ResourceInfo) -> bool {
    info.is_success() || (info.status == 416 && info.content_length.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_parsing() {
        assert_eq!(
            parse_retry_after(" 1 "),
            Some(std::time::Duration::from_secs(1))
        );
        assert_eq!(
            parse_retry_after("120"),
            Some(std::time::Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after("soon"), None);
    }

    #[test]
    fn http_date_retry_after() {
        // IMF-fixdate (RFC 9110 §5.6.7); past dates → retry immediately.
        // Anchors (verified): 2026-11-18T00:00:00Z == 1_794_960_000
        let now = 1_794_995_970_u64; // 2026-11-18T09:59:30Z
        assert_eq!(
            parse_retry_after_at("Wed, 18 Nov 2026 10:00:00 GMT", now),
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after_at("Wed, 18 Nov 2026 09:00:00 GMT", now),
            Some(std::time::Duration::from_secs(0)),
            "date in the past → retry immediately"
        );
        assert_eq!(
            parse_retry_after_at("Wed, 18 Nov 2026 00:00:00 GMT", 0),
            Some(std::time::Duration::from_secs(1_794_960_000))
        );
        // obsolete forms and garbage → None (backoff), documented deviation
        assert_eq!(
            parse_retry_after_at("Sunday, 06-Nov-94 08:49:37 GMT", now),
            None
        );
        assert_eq!(parse_retry_after_at("Sun Nov  6 08:49:37 1994", now), None);
        assert_eq!(
            parse_retry_after_at("Wed, 18 Nov 2026 10:00:00 EST", now),
            None
        );
    }

    #[test]
    fn sniff_confidently_detects_html_documents() {
        let html = b"<!DOCTYPE html><html><head><title>x</title></head><body></body></html>";
        assert_eq!(sniff_kind(html), SniffedKind::Html);
        assert_eq!(sniff_kind(b"<html lang=\"en\">"), SniffedKind::Html);
        assert_eq!(
            sniff_kind(b"\xEF\xBB\xBF<html>"),
            SniffedKind::Html,
            "BOM skipped"
        );
        assert_eq!(sniff_kind(b"\r\n  <script>"), SniffedKind::Html);
        assert_eq!(sniff_kind(b"<HTML>"), SniffedKind::Html, "case-insensitive");
        // an HTML fragment with no top-level marker must not be trusted either way
        assert_eq!(sniff_kind(b"hello <b>world</b>"), SniffedKind::Unknown);
        assert_eq!(sniff_kind(b""), SniffedKind::Unknown);
    }

    #[test]
    fn sniff_never_flags_real_binary_files_as_html() {
        // a truncated marker ("<h") is not evidence
        assert_eq!(sniff_kind(b"<h"), SniffedKind::Unknown);
        // media headers: NUL bytes → binary, never html
        let mut mp4 = b"\x00\x00\x00\x18ftypmp42".to_vec();
        mp4.resize(mp4.len() + 512, 0u8);
        assert_eq!(sniff_kind(&mp4), SniffedKind::Binary, "NUL bytes → binary");
        let zip: Vec<u8> = [0x50u8, 0x4B, 0x03, 0x04]
            .into_iter()
            .chain((0u8..240).map(|i| i.wrapping_mul(7)))
            .collect();
        assert_eq!(
            sniff_kind(&zip),
            SniffedKind::Binary,
            "PK header + binary noise"
        );
        // PDF: short high-byte sample has no confident verdict, but never Html
        let pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3";
        assert_eq!(
            sniff_kind(pdf),
            SniffedKind::Unknown,
            "short sample: no verdict"
        );
    }
}
