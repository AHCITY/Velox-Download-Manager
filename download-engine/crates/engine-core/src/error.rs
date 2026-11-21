//! Structured error model with classification (docs/PROTOCOL_COMPATIBILITY.md §5).

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidUrl,
    InvalidConfig,
    Dns,
    Connect,
    Timeout,
    Reset,
    /// HTTP status outside 2xx/3xx handling; first field is the status.
    Http(u16),
    TlsVerification,
    AuthRequired,
    AuthFailed,
    /// Server answered a ranged request wrongly (200 full body, mismatched Content-Range).
    RangeMismatch,
    /// Resource identity changed per validators.
    ResourceChanged,
    /// Framing faults (CL+TE, invalid Content-Length, ...).
    InvalidResponse,
    /// The server returned a hypertext document where a downloadable file was expected
    /// (watch pages, Drive interstitials, error pages with HTTP 200).
    NotAFile,
    /// The URL is a media page/stream that generic HTTP cannot fetch; an external
    /// extractor (yt-dlp) or a direct media URL is required.
    UnsupportedUrl,
    DiskFull,
    Permission,
    FileLock,
    Io,
    Cancelled,
}

impl ErrorKind {
    pub fn code(&self) -> &'static str {
        match self {
            ErrorKind::InvalidUrl => "INVALID_URL",
            ErrorKind::InvalidConfig => "INVALID_CONFIG",
            ErrorKind::Dns => "DNS",
            ErrorKind::Connect => "CONNECT",
            ErrorKind::Timeout => "TIMEOUT",
            ErrorKind::Reset => "RESET",
            ErrorKind::Http(_) => "HTTP",
            ErrorKind::TlsVerification => "TLS_VERIFICATION",
            ErrorKind::AuthRequired => "AUTH_REQUIRED",
            ErrorKind::AuthFailed => "AUTH_FAILED",
            ErrorKind::RangeMismatch => "RANGE_MISMATCH",
            ErrorKind::ResourceChanged => "RESOURCE_CHANGED",
            ErrorKind::InvalidResponse => "INVALID_RESPONSE",
            ErrorKind::NotAFile => "NOT_A_FILE",
            ErrorKind::UnsupportedUrl => "UNSUPPORTED_URL",
            ErrorKind::DiskFull => "DISK_FULL",
            ErrorKind::Permission => "PERMISSION",
            ErrorKind::FileLock => "FILE_LOCK",
            ErrorKind::Io => "IO",
            ErrorKind::Cancelled => "CANCELLED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Never retry.
    Permanent,
    /// Retry with backoff from the segment checkpoint.
    Temporary,
    /// Credentials needed/failed — actionable, not retried blindly.
    Auth,
    /// Validators say the remote bytes changed: restart from zero (bounded).
    Restart,
}

#[derive(Debug, Clone)]
pub struct EngineError {
    pub kind: ErrorKind,
    pub message: String,
    /// Retry-After from the server, when present.
    pub retry_after: Option<Duration>,
    pub segment_idx: Option<i64>,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind_code(), self.message)
    }
}

impl std::error::Error for EngineError {}

impl EngineError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: None,
            segment_idx: None,
        }
    }

    pub fn with_retry_after(mut self, d: Duration) -> Self {
        self.retry_after = Some(d);
        self
    }

    pub fn with_segment(mut self, idx: i64) -> Self {
        self.segment_idx = Some(idx);
        self
    }

    pub fn kind_code(&self) -> String {
        match self.kind {
            ErrorKind::Http(s) => format!("HTTP_{s}"),
            other => other.code().to_string(),
        }
    }

    pub fn classify(&self) -> ErrorClass {
        match self.kind {
            ErrorKind::InvalidUrl
            | ErrorKind::InvalidConfig
            | ErrorKind::TlsVerification
            | ErrorKind::AuthFailed
            | ErrorKind::InvalidResponse
            | ErrorKind::NotAFile
            | ErrorKind::UnsupportedUrl
            | ErrorKind::DiskFull
            | ErrorKind::Permission
            | ErrorKind::FileLock => ErrorClass::Permanent,
            ErrorKind::AuthRequired => ErrorClass::Auth,
            ErrorKind::RangeMismatch | ErrorKind::ResourceChanged | ErrorKind::Cancelled => {
                ErrorClass::Restart
            }
            ErrorKind::Http(s) => match s {
                // curl's transient taxonomy + RFC 6585/9110: 408, 429, 5xx are retryable.
                408 | 429 => ErrorClass::Temporary,
                500..=599 => ErrorClass::Temporary,
                _ => ErrorClass::Permanent,
            },
            ErrorKind::Dns
            | ErrorKind::Connect
            | ErrorKind::Timeout
            | ErrorKind::Reset
            | ErrorKind::Io => ErrorClass::Temporary,
        }
    }
}

/// Map an unexpected (non-success, non-redirect) HTTP status.
pub fn from_status(status: u16, message: impl Into<String>) -> EngineError {
    let kind = match status {
        401 | 407 => ErrorKind::AuthRequired,
        416 => ErrorKind::RangeMismatch,
        _ => ErrorKind::Http(status),
    };
    EngineError::new(kind, format!("HTTP {status}: {}", message.into()))
}

/// Map an I/O failure from the storage layer (write/read/path).
pub fn from_io(e: &std::io::Error) -> EngineError {
    let kind = match e.raw_os_error() {
        Some(28) => ErrorKind::DiskFull, // ENOSPC (POSIX)
        #[cfg(windows)]
        Some(112) | Some(39) => ErrorKind::DiskFull, // ERROR_DISK_FULL / ERROR_HANDLE_DISK_FULL
        Some(13) => ErrorKind::Permission, // EACCES / EPERM
        Some(11) | Some(4) => ErrorKind::Io, // EAGAIN / EINTR — transient
        Some(17) => ErrorKind::FileLock, // EEXIST / busy file
        Some(2) => ErrorKind::InvalidConfig, // ENOENT — path vanished: configuration-level
        _ => ErrorKind::Io,
    };
    EngineError::new(kind, format!("io: {e}"))
}

/// Map a reqwest-style transport error message after the backend classified it.
pub fn transport(kind: ErrorKind, message: impl Into<String>) -> EngineError {
    EngineError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert_eq!(from_status(404, "x").classify(), ErrorClass::Permanent);
        assert_eq!(from_status(403, "x").classify(), ErrorClass::Permanent);
        assert_eq!(from_status(429, "x").classify(), ErrorClass::Temporary);
        assert_eq!(from_status(408, "x").classify(), ErrorClass::Temporary);
        assert_eq!(from_status(503, "x").classify(), ErrorClass::Temporary);
        assert_eq!(from_status(500, "x").classify(), ErrorClass::Temporary);
        assert_eq!(from_status(416, "x").classify(), ErrorClass::Restart);
        assert_eq!(from_status(401, "x").classify(), ErrorClass::Auth);
        let e = EngineError::new(ErrorKind::DiskFull, "no space");
        assert_eq!(e.classify(), ErrorClass::Permanent);
        assert_eq!(e.kind_code(), "DISK_FULL");
        assert_eq!(from_status(502, "x").kind_code(), "HTTP_502");
    }

    #[test]
    fn io_mapping() {
        let enospc = std::io::Error::from_raw_os_error(28);
        assert_eq!(from_io(&enospc).kind, ErrorKind::DiskFull);
        let eacces = std::io::Error::from_raw_os_error(13);
        assert_eq!(from_io(&eacces).kind, ErrorKind::Permission);
        assert_eq!(from_io(&std::io::Error::other("boom")).kind, ErrorKind::Io);
    }
}
