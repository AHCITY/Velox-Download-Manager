//! Token-bucket bandwidth limiter (per download; pacing happens in the worker read loop).

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct BandwidthLimiter {
    rate_bps: u64,
    state: Mutex<LimiterState>,
}

struct LimiterState {
    last: Instant,
    budget: f64,
}

impl BandwidthLimiter {
    pub fn new(rate_bps: u64) -> Self {
        Self {
            rate_bps: rate_bps.max(1),
            state: Mutex::new(LimiterState {
                last: Instant::now(),
                budget: rate_bps as f64,
            }),
        }
    }

    /// Wait until `n` bytes may be consumed. Requests larger than one second's worth of
    /// bytes are consumed in slices — the bucket caps at `rate_bps`, so a 64 KiB chunk on an
    /// 8 KiB/s limit would otherwise spin forever (audit H1: infinite sleep outside any
    /// stop-check). Callers select this against their stop signal so pacing is cancellable.
    pub async fn acquire(&self, n: u64) {
        let mut remaining = n.max(1);
        while remaining > 0 {
            let take = remaining.min(self.rate_bps) as f64;
            let wait = {
                let mut st = self.state.lock().unwrap();
                let now = Instant::now();
                let elapsed = now.duration_since(st.last).as_secs_f64();
                st.last = now;
                st.budget = (st.budget + elapsed * self.rate_bps as f64).min(self.rate_bps as f64);
                if st.budget >= take {
                    st.budget -= take;
                    None
                } else {
                    Some(Duration::from_secs_f64(
                        (take - st.budget) / self.rate_bps as f64,
                    ))
                }
            };
            match wait {
                None => remaining -= take as u64,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn oversized_request_does_not_hang() {
        // audit H1 regression: n > rate must complete by slicing, not spin forever.
        // 300 KB at 100 kB/s = 1 s burst budget + 2 s paced ≈ 2 s total.
        let l = BandwidthLimiter::new(100_000);
        let t0 = std::time::Instant::now();
        l.acquire(300_000).await;
        let elapsed = t0.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1500),
            "pacing lost: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "oversized request hung: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn burst_within_rate_is_instant() {
        let l = BandwidthLimiter::new(1_000_000);
        let t0 = std::time::Instant::now();
        l.acquire(500_000).await; // half a second of budget: no wait needed
        assert!(t0.elapsed() < Duration::from_millis(100));
    }
}
