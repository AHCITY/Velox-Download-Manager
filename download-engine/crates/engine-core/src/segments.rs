//! Live segment table: the single source of truth for in-flight ranges and positions.
//! std Mutex only — never held across .await. Work stealing = tail splitting (ADR-0006).

use std::sync::Mutex;

use tokio::sync::Notify;

#[derive(Debug, Clone)]
pub struct SegInit {
    pub start: u64,
    /// Inclusive end; None = unbounded stream.
    pub end: Option<u64>,
    /// Resumed position (durable checkpoint) if any.
    pub pos: u64,
    pub completed: bool,
}

#[derive(Debug, Clone)]
pub struct Work {
    pub idx: usize,
    /// Absolute start offset for this work unit.
    pub start: u64,
    /// Inclusive end (None = unbounded).
    pub end: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SegSnapshot {
    pub idx: i64,
    pub start: u64,
    pub end: Option<u64>,
    pub pos: u64,
    pub completed: bool,
}

#[derive(Debug, Default, Clone)]
pub struct TableStats {
    pub received: u64,
    pub completed: usize,
    pub count: usize,
    pub active: usize,
}

#[derive(Debug)]
struct Seg {
    start: u64,
    end: Option<u64>,
    pos: u64,
    completed: bool,
    dirty: bool,
    owner: Option<u64>,
}

pub struct SegmentTable {
    inner: Mutex<Vec<Seg>>,
    notify: Notify,
    min_split: u64,
}

impl SegmentTable {
    pub fn new(segs: Vec<SegInit>, min_split: u64) -> Self {
        let inner = segs
            .into_iter()
            .map(|s| {
                let mut seg = Seg {
                    start: s.start,
                    end: s.end,
                    pos: s.pos.min(remaining_of(s.start, s.end, s.pos)),
                    completed: s.completed,
                    dirty: false,
                    owner: None,
                };
                // normalize: fully written but unmarked (interrupted at the exact boundary)
                if let Some(e) = seg.end {
                    if seg.start + seg.pos > e {
                        seg.completed = true;
                    }
                }
                seg
            })
            .collect();
        Self {
            inner: Mutex::new(inner),
            notify: Notify::new(),
            min_split: min_split.max(1),
        }
    }

    /// Assign next work to a worker: an unassigned incomplete segment, or the stolen tail of the
    /// largest active segment when `remaining ≥ 2 × min_split`. Owners clamp every write under the
    /// same lock, so a stolen range can never be double-written.
    pub fn take(&self, worker: u64) -> Option<Work> {
        let mut segs = self.inner.lock().unwrap();
        // normalize first: a segment written to its end is complete, whatever its flag says
        for s in segs.iter_mut() {
            if !s.completed {
                if let Some(e) = s.end {
                    if s.start + s.pos > e {
                        s.completed = true;
                        s.dirty = true;
                    }
                }
            }
        }
        if let Some((idx, s)) = segs
            .iter_mut()
            .enumerate()
            .find(|(_, s)| !s.completed && s.owner.is_none())
        {
            s.owner = Some(worker);
            return Some(Work {
                idx,
                start: s.start + s.pos,
                end: s.end,
            });
        }
        let mut best: Option<(usize, u64)> = None;
        for (i, s) in segs.iter().enumerate() {
            if s.completed || s.owner.is_none() {
                continue;
            }
            if let Some(end) = s.end {
                let rem = end + 1 - (s.start + s.pos);
                if rem >= 2 * self.min_split && best.is_none_or(|(_, r)| rem > r) {
                    best = Some((i, rem));
                }
            }
        }
        if let Some((i, rem)) = best {
            let (split_at, old_end) = {
                let v = &mut segs[i];
                let cur = v.start + v.pos;
                let split_at = cur + rem / 2;
                let old_end = v.end.unwrap();
                v.end = Some(split_at - 1);
                (split_at, old_end)
            };
            segs.push(Seg {
                start: split_at,
                end: Some(old_end),
                pos: 0,
                completed: false,
                dirty: false,
                owner: Some(worker),
            });
            let idx = segs.len() - 1;
            return Some(Work {
                idx,
                start: split_at,
                end: Some(old_end),
            });
        }
        None
    }

    /// Clamp a chunk against the (possibly stolen-shrunk) segment; returns (absolute offset, allowed bytes).
    /// None when the segment is completed or the clamp leaves zero bytes (caller must close the connection).
    pub fn span(&self, idx: usize, chunk_len: usize) -> Option<(u64, usize)> {
        let segs = self.inner.lock().unwrap();
        let s = segs.get(idx)?;
        if s.completed {
            return None;
        }
        let wanted = chunk_len as u64;
        let rem = match s.end {
            Some(e) => (e + 1).saturating_sub(s.start + s.pos),
            None => wanted,
        };
        let allowed = wanted.min(rem) as usize;
        if allowed == 0 {
            return None;
        }
        Some((s.start + s.pos, allowed))
    }

    /// Record a successful positional write.
    pub fn advance(&self, idx: usize, n: u64) {
        let mut segs = self.inner.lock().unwrap();
        if let Some(s) = segs.get_mut(idx) {
            let cap = match s.end {
                Some(e) => e + 1 - s.start,
                None => u64::MAX,
            };
            s.pos = (s.pos + n).min(cap);
            s.dirty = true;
        }
    }

    /// Segment finished (all its bytes written). Releases ownership; wakes parked workers.
    pub fn complete(&self, idx: usize) {
        let mut segs = self.inner.lock().unwrap();
        if let Some(s) = segs.get_mut(idx) {
            s.completed = true;
            if let Some(e) = s.end {
                s.pos = e + 1 - s.start;
            }
            s.dirty = true;
            s.owner = None;
        }
        drop(segs);
        self.notify.notify_waiters();
    }

    /// Release ownership without completing (pause/stop/restart).
    pub fn abandon(&self, idx: usize) {
        let mut segs = self.inner.lock().unwrap();
        if let Some(s) = segs.get_mut(idx) {
            s.owner = None;
        }
        drop(segs);
        self.notify.notify_waiters();
    }

    /// Full snapshot for persistence (task replaces DB rows transactionally — always consistent).
    pub fn snapshot(&self) -> Vec<SegSnapshot> {
        let segs = self.inner.lock().unwrap();
        segs.iter()
            .enumerate()
            .map(|(idx, s)| SegSnapshot {
                idx: idx as i64,
                start: s.start,
                end: s.end,
                pos: s.pos,
                completed: s.completed,
            })
            .collect()
    }

    pub fn stats(&self) -> TableStats {
        let segs = self.inner.lock().unwrap();
        TableStats {
            received: segs.iter().map(|s| s.pos).sum(),
            completed: segs.iter().filter(|s| s.completed).count(),
            count: segs.len(),
            active: segs
                .iter()
                .filter(|s| s.owner.is_some() && !s.completed)
                .count(),
        }
    }

    pub fn all_done(&self) -> bool {
        self.inner.lock().unwrap().iter().all(|s| s.completed)
    }

    /// Bytes still to fetch (unbounded segments count as unknown → 0 contributes nothing).
    pub fn remaining(&self) -> Option<u64> {
        let segs = self.inner.lock().unwrap();
        let mut acc = 0u64;
        for s in segs.iter() {
            {
                let e = s.end?;
                acc += e + 1 - (s.start + s.pos)
            }
        }
        Some(acc)
    }

    /// Current durable position of a segment (absolute bytes written within it).
    pub fn pos_of(&self, idx: usize) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .get(idx)
            .map(|s| s.pos)
            .unwrap_or(0)
    }

    /// Live (possibly clamped) inclusive end of a segment — always fresh for range requests.
    pub fn end_of(&self, idx: usize) -> Option<u64> {
        self.inner
            .lock()
            .unwrap()
            .get(idx)
            .map(|s| s.end)
            .unwrap_or(None)
    }

    /// Absolute start offset of the current work unit for a segment (start + pos).
    pub fn abs_pos(&self, idx: usize) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .get(idx)
            .map(|s| s.start + s.pos)
            .unwrap_or(0)
    }

    /// True when at least one segment has uncheckpointed progress.
    pub fn has_dirty(&self) -> bool {
        self.inner.lock().unwrap().iter().any(|s| s.dirty)
    }

    /// Wait for a state change (completion/abandon) with a poll timeout handled by the caller.
    pub async fn wait_change(&self) {
        self.notify.notified().await;
    }
}

fn remaining_of(start: u64, end: Option<u64>, pos: u64) -> u64 {
    match end {
        Some(e) => pos.min(e + 1 - start),
        None => pos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_advance_complete() {
        let t = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(99),
                pos: 0,
                completed: false,
            }],
            16,
        );
        let w = t.take(1).unwrap();
        assert_eq!((w.idx, w.start, w.end), (0, 0, Some(99)));
        assert_eq!(t.span(0, 30), Some((0, 30)));
        t.advance(0, 30);
        assert_eq!(t.span(0, 500), Some((30, 70)), "clamped to segment end");
        t.advance(0, 70);
        t.complete(0);
        assert!(t.all_done());
        assert_eq!(t.stats().received, 100);
        assert!(t.span(0, 10).is_none(), "completed segment yields nothing");
    }

    #[test]
    fn stealing_splits_largest_tail() {
        // one big active segment (worker 1), worker 2 arrives with empty queue
        let t = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(999),
                pos: 0,
                completed: false,
            }],
            100,
        );
        let w1 = t.take(1).unwrap();
        t.advance(w1.idx, 200); // worker1 at 0..200 done
        let w2 = t.take(2).unwrap();
        assert_eq!(w2.idx, 1, "steal creates a new segment");
        assert_eq!(w2.start, 600, "tail of the victim is stolen");
        assert_eq!(w2.end, Some(999));
        // victim is clamped: next span must not cross 600
        assert_eq!(t.span(0, 1000), Some((200, 400)));
        t.advance(0, 400);
        t.complete(0);
        t.complete(1);
        assert!(t.all_done());
        assert_eq!(t.stats().received, 1000);
    }

    #[test]
    fn steal_respects_min_split_floor() {
        let t = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(250),
                pos: 0,
                completed: false,
            }],
            100,
        );
        let w1 = t.take(1).unwrap();
        let w2 = t.take(2).unwrap();
        assert_eq!(
            w2.idx, 1,
            "251 bytes with floor 100: 2*100=200 <= 251 so a steal IS allowed"
        );
        t.abandon(w1.idx);
        t.abandon(w2.idx);
        let t2 = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(198),
                pos: 0,
                completed: false,
            }],
            100,
        );
        let a = t2.take(1).unwrap();
        assert!(
            t2.take(2).is_none(),
            "199 bytes < 2*floor → no steal, worker parks"
        );
        t2.abandon(a.idx);
    }

    #[test]
    fn unbounded_stream_no_steal() {
        let t = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: None,
                pos: 0,
                completed: false,
            }],
            16,
        );
        let w = t.take(1).unwrap();
        assert_eq!(w.end, None);
        assert!(t.take(2).is_none());
        assert_eq!(t.span(0, 50), Some((0, 50)));
        t.advance(0, 50);
        assert_eq!(t.span(0, 50), Some((50, 50)));
        t.advance(0, 50);
        t.complete(0);
        assert_eq!(t.stats().received, 100);
        assert!(t.remaining().is_none());
    }

    #[test]
    fn resume_positions() {
        let t = SegmentTable::new(
            vec![
                SegInit {
                    start: 0,
                    end: Some(499),
                    pos: 200,
                    completed: false,
                },
                SegInit {
                    start: 500,
                    end: Some(999),
                    pos: 500,
                    completed: true,
                },
            ],
            64,
        );
        let stats = t.stats();
        assert_eq!(stats.received, 700);
        assert_eq!(stats.completed, 1);
        let w = t.take(1).unwrap();
        assert_eq!(
            (w.idx, w.start),
            (0, 200),
            "resumed segment continues at its checkpoint"
        );
    }

    #[test]
    fn snapshot_roundtrip() {
        let t = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(99),
                pos: 0,
                completed: false,
            }],
            16,
        );
        let w = t.take(1).unwrap();
        t.advance(w.idx, 40);
        let snap = t.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].pos, 40);
        assert_eq!(snap[0].end, Some(99));
    }

    #[test]
    fn fully_written_unmarked_segment_normalizes() {
        // regression: a pause/abandon at the exact boundary leaves pos == length with the
        // completed flag unset; resuming must complete it, never re-request start > end
        let t = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(99),
                pos: 100,
                completed: false,
            }],
            16,
        );
        assert!(
            t.take(1).is_none(),
            "fully written segment must not be reassigned"
        );
        assert!(t.all_done());
        let snap = t.snapshot();
        assert!(
            snap[0].completed,
            "snapshot must carry the normalized completion"
        );

        // same via live advance → abandon at the boundary
        let t2 = SegmentTable::new(
            vec![SegInit {
                start: 0,
                end: Some(9),
                pos: 0,
                completed: false,
            }],
            4,
        );
        let w = t2.take(1).unwrap();
        t2.advance(w.idx, 10);
        t2.abandon(w.idx);
        assert!(
            t2.take(2).is_none(),
            "no invalid start>end request may be built"
        );
        assert!(t2.all_done());
    }
}
