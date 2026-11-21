//! Retry policy: exponential backoff with jitter, honoring Retry-After.

use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Attempts per segment/operation before the download fails.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    /// Fraction of the delay added as jitter (0.0..=1.0).
    pub jitter_ratio: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter_ratio: 0.2,
        }
    }
}

impl RetryPolicy {
    /// Retry-After wins (RFC 9110 §10.2.3) but is clamped to `max_delay`: an unbounded
    /// server-provided delay would park a worker — and its per-host permit — indefinitely
    /// (audit A5). Past/malformed dates parse to None → backoff; cap also bounds those.
    pub fn delay_for(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(ra) = retry_after {
            return ra.min(self.max_delay);
        }
        let shift = attempt.min(20);
        let exp = self.base_delay.saturating_mul(1u32 << shift);
        let d = exp.min(self.max_delay);
        let jitter_ms = (d.as_secs_f64() * self.jitter_ratio * cheap_jitter()) as u64;
        d + Duration::from_millis(jitter_ms.min(1000))
    }

    pub fn exhausted(&self, attempt: u32) -> bool {
        attempt >= self.max_attempts
    }
}

/// Cheap pseudo-random jitter in [0,1) — xorshift seeded from clock + pid (no rand dependency).
pub fn cheap_jitter() -> f64 {
    let mut s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0x9E3779B97F4A7C15)
        ^ (std::process::id() as u64) << 32;
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    (s % 10_000) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_growth_and_cap() {
        let p = RetryPolicy {
            max_attempts: 6,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter_ratio: 0.0,
        };
        assert_eq!(p.delay_for(0, None), Duration::from_secs(1));
        assert_eq!(p.delay_for(1, None), Duration::from_secs(2));
        assert_eq!(p.delay_for(2, None), Duration::from_secs(4));
        assert_eq!(p.delay_for(9, None), Duration::from_secs(60));
        let ra = Duration::from_secs(120);
        // Retry-After is honored but clamped to max_delay (audit A5): a hostile server must
        // not park a worker — and its per-host permit — longer than the policy allows
        assert_eq!(
            p.delay_for(0, Some(ra)),
            Duration::from_secs(60),
            "Retry-After above max_delay is clamped"
        );
        assert_eq!(
            p.delay_for(0, Some(Duration::from_secs(30))),
            Duration::from_secs(30),
            "Retry-After within max_delay passes through"
        );
        assert!(p.exhausted(6));
        assert!(!p.exhausted(5));
    }

    #[test]
    fn retry_after_is_clamped_to_max_delay() {
        // audit A5: a hostile server must not park a worker (and its per-host permit) forever
        let p = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter_ratio: 0.0,
        };
        assert_eq!(
            p.delay_for(0, Some(Duration::from_secs(31_536_000))),
            Duration::from_secs(60)
        );
        assert_eq!(
            p.delay_for(0, Some(Duration::from_millis(50))),
            Duration::from_millis(50),
            "small values pass through"
        );
    }
}
