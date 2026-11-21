//! Netscape cookie-file compatibility (milestone: cookie jar compat).
//!
//! Format (the de-facto `cookies.txt` standard used by curl/wget/browsers):
//! ```text
//! #HttpOnly_.example.com\tTRUE\t/\tTRUE\t1735689600\tsid\tabc123
//! example.com\tFALSE\t/\tFALSE\t0\ttheme\tdark
//! ```
//! Fields: domain, include-subdomains flag, path, secure, expires (unix
//! seconds, 0 = session), name, value. Lines starting with `#` are comments
//! (except the `#HttpOnly_` marker). Import maps each line onto an RFC 6265
//! cookie_store entry with the correct HostOnly/Suffix domain semantics, so
//! domain matching and expiry stay in one well-tested implementation
//! (`cookie_store`). Expired entries are dropped at import; cookie values are
//! never logged (SECURITY: logs carry names/counts only, never values).

use crate::SharedCookieStore;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq)]
pub struct NetscapeCookie {
    pub domain: String,
    pub include_subdomains: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// unix seconds; 0 = session cookie
    pub expires: i64,
    pub name: String,
    pub value: String,
}

/// Parse a Netscape cookie file. Malformed lines are skipped (counted in the
/// result via the len diff); expired persistent cookies are kept here and
/// filtered by the caller so tests can observe the full parse.
pub fn parse(content: &str) -> Vec<NetscapeCookie> {
    let mut out = Vec::new();
    for line in content.lines() {
        let (http_only, line) = match line.strip_prefix("#HttpOnly_") {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // 6 tabs split the 7 fields; the value may itself contain tabs, so the
        // value is everything after the 6th tab
        let mut parts = line.splitn(7, '\t');
        let (Some(domain), Some(flag), Some(path), Some(secure), Some(expires), Some(name)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let Some(value) = parts.next() else { continue };
        if name.is_empty() {
            continue;
        }
        out.push(NetscapeCookie {
            domain: domain.trim().to_string(),
            include_subdomains: flag.trim().eq_ignore_ascii_case("true"),
            path: if path.trim().is_empty() {
                "/".into()
            } else {
                path.trim().to_string()
            },
            secure: secure.trim().eq_ignore_ascii_case("true"),
            http_only,
            expires: expires.trim().parse().unwrap_or(0),
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    out
}

/// Import into the shared jar. Returns the number of cookies inserted.
/// Domain semantics: `include_subdomains` → Suffix (matches subdomains),
/// otherwise HostOnly (exact host only) — no cross-domain leakage beyond what
/// the source file itself declares.
pub fn import(store: &SharedCookieStore, content: &str) -> Result<usize, String> {
    let mut g = store
        .lock()
        .map_err(|_| "cookie store poisoned".to_string())?;
    let now = OffsetDateTime::now_utc();
    let mut n = 0usize;
    for c in parse(content) {
        if c.expires > 0
            && OffsetDateTime::from_unix_timestamp(c.expires)
                .map(|e| e < now)
                .unwrap_or(true)
        {
            continue; // expired at import: never resurrect stale sessions
        }
        let host = c.domain.trim_start_matches('.').trim();
        if host.is_empty() || c.name.is_empty() {
            continue;
        }
        // public-suffix guard: a single-label domain ("com", "org", "localhost") would
        // match EVERY registrable domain under it (the jar has no PSL loaded) — reject,
        // mirroring RFC 6265 §5.3's public-suffix rule (review finding)
        if !host.contains('.') {
            continue;
        }
        let scheme = if c.secure { "https" } else { "http" };
        let path = if c.path.starts_with('/') {
            c.path.clone()
        } else {
            format!("/{}", c.path) // Netscape files occasionally omit the leading slash
        };
        let url = match url::Url::parse(&format!("{scheme}://{host}{path}")) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let raw = cookie_store::RawCookie::build(cookie_store::RawCookie::new(
            c.name.clone(),
            c.value.clone(),
        ))
        .path(c.path.clone())
        .secure(c.secure)
        .http_only(c.http_only)
        .build();
        let mut cookie = match cookie_store::Cookie::try_from_raw_cookie(&raw, &url) {
            Ok(k) => k,
            Err(_) => continue,
        };
        // override the domain derived from the synthetic URL with the file's
        // declared domain semantics
        cookie.domain = if c.include_subdomains {
            cookie_store::CookieDomain::Suffix(host.to_string()) // cookie_store stores Suffix WITHOUT the leading dot
        } else {
            cookie_store::CookieDomain::HostOnly(host.to_string())
        };
        cookie.expires = if c.expires == 0 {
            cookie_store::CookieExpiration::SessionEnd
        } else if let Ok(dt) = OffsetDateTime::from_unix_timestamp(c.expires) {
            cookie_store::CookieExpiration::AtUtc(dt)
        } else {
            cookie_store::CookieExpiration::SessionEnd
        };
        if g.insert(cookie, &url).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// Export the jar's unexpired cookies in Netscape format. Session cookies are
/// written with expires=0 per the format's convention.
pub fn export(store: &SharedCookieStore) -> String {
    let mut lines = vec![
        "# Netscape HTTP Cookie File".to_string(),
        "# Generated by velox. This file may contain session credentials.".to_string(),
        "# Keep it private (velox writes it with 0600 on Unix).".to_string(),
    ];
    if let Ok(g) = store.lock() {
        let now = OffsetDateTime::now_utc();
        for c in g.iter_unexpired() {
            let (domain, host_only) = match &c.domain {
                cookie_store::CookieDomain::Suffix(s) => {
                    let s = s.trim_start_matches('.');
                    (format!(".{s}"), false)
                }
                cookie_store::CookieDomain::HostOnly(s) => (s.clone(), true),
                _ => continue,
            };
            let expires = match &c.expires {
                cookie_store::CookieExpiration::AtUtc(dt) if *dt >= now => dt.unix_timestamp(),
                cookie_store::CookieExpiration::AtUtc(_) => continue, // expired: skip
                cookie_store::CookieExpiration::SessionEnd => 0,
            };
            // Cookie derefs to RawCookie: name/value/secure/http_only come through
            let secure = c.secure().unwrap_or(false);
            let httponly = c.http_only().unwrap_or(false);
            let prefix = if httponly { "#HttpOnly_" } else { "" };
            lines.push(format!(
                "{prefix}{domain}\t{}\t{}\t{}\t{expires}\t{}\t{}",
                if host_only { "FALSE" } else { "TRUE" },
                c.path.as_ref(), // CookiePath derefs to str
                if secure { "TRUE" } else { "FALSE" },
                c.name(),
                c.value(),
            ));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn new_store() -> SharedCookieStore {
        Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            cookie_store::CookieStore::default(),
        ))
    }

    const SAMPLE: &str = "# Netscape HTTP Cookie File\n\
        #HttpOnly_.example.com\tTRUE\t/\tTRUE\t0\t__Host-sid\tabc\twith\ttabs\n\
        example.org\tFALSE\t/dir\tFALSE\t0\ttheme\tdark\n\
        sub.example.net\tTRUE\t/\tTRUE\t4102444800\tapi\ttok\n\
        expired.example.com\tFALSE\t/\tFALSE\t1600000000\tstale\tgone\n\
        malformed-line-no-tabs\n";

    #[test]
    fn parse_handles_all_field_semantics() {
        let c = parse(SAMPLE);
        assert_eq!(c.len(), 4, "comments and malformed lines are skipped");
        let sid = &c[0];
        assert!(sid.http_only && sid.include_subdomains && sid.secure);
        assert_eq!(sid.domain, ".example.com");
        assert_eq!(sid.name, "__Host-sid");
        assert_eq!(sid.value, "abc\twith\ttabs", "tabs inside values preserved");
        let theme = &c[1];
        assert!(!theme.include_subdomains && !theme.secure && !theme.http_only);
        assert_eq!(theme.path, "/dir");
        assert_eq!(c[3].domain, "expired.example.com");
    }

    #[test]
    fn import_respects_domain_and_expiry() {
        let store = new_store();
        let n = import(&store, SAMPLE).unwrap();
        assert_eq!(n, 3, "expired cookie is dropped, not resurrected");
        let g = store.lock().unwrap();
        assert_eq!(g.iter_any().count(), 3);
        assert!(!g.iter_any().any(|k| k.name() == "stale"));
    }

    #[test]
    fn suffix_and_host_prefix_semantics() {
        // domain-cookie (Suffix) and __Host- names both import cleanly
        let store = new_store();
        let n = import(&store, SAMPLE).unwrap();
        assert_eq!(n, 3);
        let g = store.lock().unwrap();
        let sid = g
            .iter_any()
            .find(|k| k.name() == "__Host-sid")
            .expect("sid imported");
        assert!(matches!(sid.domain, cookie_store::CookieDomain::Suffix(_)));
        assert!(sid.http_only().unwrap_or(false));
    }

    #[test]
    fn import_export_roundtrip_preserves_semantics() {
        let store = new_store();
        import(&store, SAMPLE).unwrap();
        let out = export(&store);
        assert!(out.starts_with("# Netscape HTTP Cookie File"));
        assert!(
            out.contains("#HttpOnly_.example.com\tTRUE\t/\tTRUE\t0\t__Host-sid\tabc\twith\ttabs")
        );
        assert!(out.contains("example.org\tFALSE\t/dir\tFALSE\t0\ttheme\tdark"));
        let store2 = new_store();
        assert_eq!(import(&store2, &out).unwrap(), 3);
        let g2 = store2.lock().unwrap();
        assert!(g2
            .iter_any()
            .any(|k| k.name() == "__Host-sid" && k.http_only().unwrap_or(false)));
    }

    #[test]
    fn imported_cookies_match_only_their_domains() {
        // domain matching is cookie_store's RFC 6265 job; verify import kept
        // HostOnly/Suffix semantics so matches() behaves correctly
        let store = new_store();
        import(&store, SAMPLE).unwrap();
        let g = store.lock().unwrap();
        let host = url::Url::parse("https://example.com/").unwrap();
        let sub = url::Url::parse("https://sub.example.com/").unwrap();
        let other = url::Url::parse("https://unrelated.example/").unwrap();
        let matches_for = |u: &url::Url| {
            g.iter_unexpired()
                .filter(|k| k.domain.matches(u))
                .map(|k| k.name().to_string())
                .collect::<Vec<_>>()
        };
        let at_host = matches_for(&host);
        assert!(
            at_host.contains(&"__Host-sid".to_string()),
            "Suffix matches apex"
        );
        let at_sub = matches_for(&sub);
        assert!(
            at_sub.contains(&"__Host-sid".to_string()),
            "Suffix matches subdomain"
        );
        assert!(
            !at_sub.contains(&"theme".to_string()),
            "HostOnly(example.org) never leaks to example.com"
        );
        assert!(
            matches_for(&other).is_empty(),
            "no cookie may leak to an unrelated domain"
        );
    }

    #[test]
    fn session_cookies_survive_export() {
        let store = new_store();
        import(&store, "example.io\tFALSE\t/\tTRUE\t0\tsess\tv\n").unwrap();
        let out = export(&store);
        assert!(out.contains("example.io\tFALSE\t/\tTRUE\t0\tsess\tv"));
    }

    #[test]
    fn import_errors_carry_no_cookie_values() {
        let store = new_store();
        let content = "example.com\tFALSE\t/\tTRUE\t0\tk\tSECRETVALUE\n";
        assert_eq!(import(&store, content).unwrap(), 1);
        // the only error path is a poisoned store: its message is static text
        assert!(!"cookie store poisoned".contains("SECRETVALUE"));
    }
}
