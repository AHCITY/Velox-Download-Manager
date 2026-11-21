//! Download planning: how many connections, how segments are shaped (ADR-0006).

use crate::config::{DownloadConfig, EngineConfig};
use crate::inspect::{RangeSupport, ResourceInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    /// Multiple concurrent ranged segments.
    Segmented,
    /// One connection; length known → resumable if ranges supported.
    Single,
    /// One connection; length unknown (chunked/close-delimited) → non-resumable streaming.
    Stream,
}

#[derive(Debug, Clone)]
pub struct PlannedSegment {
    pub start: u64,
    pub end: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub mode: DownloadMode,
    pub total: Option<u64>,
    pub segments: Vec<PlannedSegment>,
    pub max_connections: usize,
}

pub fn effective_max_connections(cfg: &EngineConfig, dl: &DownloadConfig) -> usize {
    dl.max_connections
        .unwrap_or(cfg.max_connections_per_download)
        .clamp(1, cfg.max_connections_per_host.max(1))
}

pub fn build_plan(info: &ResourceInfo, cfg: &EngineConfig, dl: &DownloadConfig) -> Plan {
    let total = info.content_length;
    let max_conn = effective_max_connections(cfg, dl);
    let min_seg = dl.min_segment_size.unwrap_or(cfg.min_segment_size).max(1);
    let ranges_ok = info.range_support == RangeSupport::Yes;

    let (mode, segments) = match total {
        None => (
            DownloadMode::Stream,
            vec![PlannedSegment {
                start: 0,
                end: None,
            }],
        ),
        Some(0) => (DownloadMode::Single, vec![]),
        Some(t) if !ranges_ok || t < 2 * min_seg || max_conn == 1 => (
            DownloadMode::Single,
            vec![PlannedSegment {
                start: 0,
                end: Some(t.saturating_sub(1)),
            }],
        ),
        Some(t) => {
            // k0 = min(max_conn, ceil(total / (2*min_seg))) — starts conservative, grows via stealing.
            let denom = 2 * min_seg;
            let k0 = t.div_ceil(denom).max(1).min(max_conn as u64) as usize;
            let mut segments = Vec::with_capacity(k0);
            let base = t / k0 as u64;
            let rem = t % k0 as u64;
            let mut off = 0u64;
            for i in 0..k0 {
                let len = base + if (i as u64) < rem { 1 } else { 0 };
                segments.push(PlannedSegment {
                    start: off,
                    end: Some(off + len - 1),
                });
                off += len;
            }
            (DownloadMode::Segmented, segments)
        }
    };

    Plan {
        mode,
        total,
        segments,
        max_connections: max_conn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::ResourceInfo;

    fn info(len: Option<u64>, ranges: bool) -> ResourceInfo {
        ResourceInfo {
            final_url: "https://x/f.bin".into(),
            status: 200,
            content_length: len,
            range_support: if ranges {
                RangeSupport::Yes
            } else {
                RangeSupport::No
            },
            ..Default::default()
        }
    }

    fn cfg(min_seg: u64) -> EngineConfig {
        EngineConfig {
            min_segment_size: min_seg,
            ..Default::default()
        }
    }

    #[test]
    fn per_download_min_segment_override_widens_first_wave() {
        // --min-segment: 32 MiB with 8 MiB floor → k0 = ceil(32/(2*8)) = 2 segments;
        // with a 4 MiB floor → k0 = ceil(32/(2*4)) = 4 segments (benchmark follow-up)
        let mut dl = DownloadConfig {
            max_connections: Some(8),
            ..DownloadConfig::default()
        };
        let p = build_plan(
            &info(Some(32 * 1024 * 1024), true),
            &cfg(8 * 1024 * 1024),
            &dl,
        );
        assert_eq!(p.segments.len(), 2, "default floor");
        dl.min_segment_size = Some(4 * 1024 * 1024);
        let p = build_plan(
            &info(Some(32 * 1024 * 1024), true),
            &cfg(8 * 1024 * 1024),
            &dl,
        );
        assert_eq!(
            p.segments.len(),
            4,
            "overridden floor widens the first wave"
        );
        dl.min_segment_size = Some(0);
        let p = build_plan(
            &info(Some(32 * 1024 * 1024), true),
            &cfg(8 * 1024 * 1024),
            &dl,
        );
        assert_eq!(
            p.segments.len(),
            8,
            "floor clamps to 1 → capped by max_connections"
        );
    }

    #[test]
    fn unknown_length_is_stream() {
        let p = build_plan(&info(None, true), &cfg(1 << 20), &DownloadConfig::default());
        assert_eq!(p.mode, DownloadMode::Stream);
        assert_eq!(p.segments.len(), 1);
        assert!(p.segments[0].end.is_none());
    }

    #[test]
    fn small_or_unsupported_is_single() {
        let small = build_plan(
            &info(Some(100), true),
            &cfg(1 << 20),
            &DownloadConfig::default(),
        );
        assert_eq!(small.mode, DownloadMode::Single);
        let norange = build_plan(
            &info(Some(100 << 20), false),
            &cfg(1 << 20),
            &DownloadConfig::default(),
        );
        assert_eq!(norange.mode, DownloadMode::Single);
    }

    #[test]
    fn segmented_split_covers_exact_total() {
        let c = cfg(1 << 20);
        let dl = DownloadConfig {
            max_connections: Some(4),
            ..Default::default()
        };
        let p = build_plan(&info(Some(10 * (1 << 20)), true), &c, &dl);
        assert_eq!(p.mode, DownloadMode::Segmented);
        assert_eq!(p.segments.len(), 4);
        let mut sum = 0;
        for (i, s) in p.segments.iter().enumerate() {
            let end = s.end.unwrap();
            assert_eq!(s.start, sum, "contiguous segments");
            assert!(end >= s.start);
            if i + 1 < p.segments.len() {
                assert!(end - s.start + 1 >= c.min_segment_size);
            }
            sum = end + 1;
        }
        assert_eq!(sum, 10 * (1 << 20));
    }

    #[test]
    fn connection_clamp() {
        let c = cfg(1 << 20);
        let dl = DownloadConfig {
            max_connections: Some(64),
            ..Default::default()
        };
        let p = build_plan(&info(Some(4 * (1 << 30)), true), &c, &dl);
        assert!(p.segments.len() <= c.max_connections_per_host);
        let dl1 = DownloadConfig {
            max_connections: Some(1),
            ..Default::default()
        };
        let p1 = build_plan(&info(Some(4 * (1 << 30)), true), &c, &dl1);
        assert_eq!(p1.mode, DownloadMode::Single);
    }
}
// note: appended test below inside tests module via patch
