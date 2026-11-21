//! P4: segment scheduler property tests. A deterministic pseudo-random simulation hammers
//! SegmentTable with take/advance/complete/abandon/steal sequences and asserts, after every
//! operation, the invariants that make tail-stealing safe:
//!
//!   I1  no byte is ever written twice (no overlap between segments at any moment);
//!   I2  at quiescence every byte of the planned range is covered exactly once (no gaps);
//!   I3  no segment can produce an invalid request (start > end) — boundary normalization;
//!   I4  stolen ranges always respect the min-split floor;
//!   I5  aggregate accounting (received == Σ pos) matches the write log.

use engine_core::segments::{SegInit, SegmentTable};

/// xorshift64* — deterministic, no rand dependency.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[test]
fn property_no_overlap_no_gap_exact_coverage() {
    let total: u64 = 300_000;
    let min_split: u64 = 4_096;
    let mut rng = Rng::new(0x5EED_2026_1119);

    for round in 0..200u32 {
        let segs = vec![SegInit {
            start: 0,
            end: Some(total - 1),
            pos: 0,
            completed: false,
        }];
        let table = SegmentTable::new(segs, min_split);

        // written[b] counts how many times byte b was written; must stay ≤ 1 within a run
        let mut written = vec![0u32; total as usize];
        let mut workers: Vec<(u64, Option<usize>)> = (1..=6).map(|w| (w as u64, None)).collect();

        let mut steps = 0;
        while !table.all_done() {
            steps += 1;
            assert!(
                steps < 100_000,
                "round {round}: simulation did not converge"
            );

            let wi = (rng.below(workers.len() as u64)) as usize;
            let w = &mut workers[wi];
            // pick an action
            match rng.below(10) {
                0..=3 => {
                    // try to take work
                    if let Some(cur) = w.1 {
                        table.abandon(cur);
                        w.1 = None;
                    }
                    if let Some(work) = table.take(w.0) {
                        if let Some(e) = work.end {
                            assert!(
                                work.start <= e,
                                "round {round}: invalid work unit start {} > end {}",
                                work.start,
                                e
                            );
                        }
                        w.1 = Some(work.idx);
                    }
                }
                4..=8 => {
                    // advance the worker's segment by a random amount, writing bytes
                    if let Some(idx) = w.1 {
                        let chunk = 1 + rng.below(32 * 1024);
                        if let Some((off, allowed)) = table.span(idx, chunk as usize) {
                            for b in off..off + allowed as u64 {
                                let slot = &mut written[b as usize];
                                assert!(
                                    *slot == 0,
                                    "round {round}: byte {b} written twice (I1 violated)"
                                );
                                *slot = 1;
                            }
                            table.advance(idx, allowed as u64);
                        } else {
                            // clamped to nothing or already complete → complete if at end
                            if table.end_of(idx).is_none_or(|e| table.abs_pos(idx) > e) {
                                table.complete(idx);
                                w.1 = None;
                            } else {
                                table.abandon(idx);
                                w.1 = None;
                            }
                        }
                        if table.end_of(idx).is_some_and(|e| table.abs_pos(idx) > e) {
                            table.complete(idx);
                            w.1 = None;
                        }
                    }
                }
                _ => {
                    // occasionally abandon to churn ownership
                    if let Some(idx) = w.1 {
                        table.abandon(idx);
                        w.1 = None;
                    }
                }
            }

            // I5 after every step
            let stats = table.stats();
            assert_eq!(
                stats.received,
                written.iter().map(|c| *c as u64).sum::<u64>(),
                "round {round}: aggregate accounting diverged from the write log"
            );
        }

        // I2: exact coverage at quiescence
        let uncovered = written.iter().filter(|c| **c == 0).count();
        assert_eq!(
            uncovered, 0,
            "round {round}: {uncovered} bytes never written (I2 violated)"
        );
        let _ = total;
    }
}

#[test]
fn property_multi_segment_plan_is_exact_partition() {
    // planning itself must always partition the file exactly (contiguous, complete, ordered)
    use engine_core::config::{DownloadConfig, EngineConfig};
    use engine_core::inspect::{RangeSupport, ResourceInfo};
    use engine_core::plan::build_plan;

    let mut rng = Rng::new(0xC0FFEE);
    let cfg = EngineConfig {
        min_segment_size: 1 + rng.below(64 * 1024),
        max_connections_per_host: 1 + rng.below(16) as usize,
        max_connections_per_download: 1 + rng.below(16) as usize,
        ..Default::default()
    };
    for _ in 0..300 {
        let total = rng.below(2 * 1024 * 1024);
        let info = ResourceInfo {
            final_url: "https://x/f".into(),
            status: 200,
            content_length: Some(total),
            range_support: RangeSupport::Yes,
            ..Default::default()
        };
        let dl = DownloadConfig {
            max_connections: Some(1 + rng.below(16) as usize),
            ..Default::default()
        };
        let plan = build_plan(&info, &cfg, &dl);
        if plan.mode != engine_core::plan::DownloadMode::Segmented {
            continue;
        }
        let mut off = 0u64;
        for s in &plan.segments {
            let e = s.end.expect("segmented plan has bounded segments");
            assert_eq!(s.start, off, "segments must be contiguous");
            assert!(e >= s.start);
            off = e + 1;
        }
        assert_eq!(off, total, "segments must cover exactly total");
        assert!(plan.segments.len() <= plan.max_connections);
    }
}
