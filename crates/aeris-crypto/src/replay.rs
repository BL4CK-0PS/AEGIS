//! Per-source sequence numbers and the replay window (§6.2).
//!
//! ```text
//! accept(e) ⇔ V(e) = 1 ∧ seq_e > last[source_e] ∧ |t_e − t_now| ≤ Δmax
//! ```
//!
//! `Δmax = 2 s`. Events flagged `replayed` (sent from a partition buffer, S7)
//! are judged on sequence number alone, with two bounds the formula leaves
//! open: they may not be from the future (`t_e ≤ t_now + Δmax`) and may not be
//! older than `replay_max_age` (default 300 s). The flag is inside the signed
//! body, so only the key holder can set it, and `seq > last` still applies:
//! a captured old event can never be re-admitted.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Sender-side per-source sequence counter. Starts at 1, never wraps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceCounter {
    last: u64,
}

/// The counter reached `u64::MAX`; the source must be re-keyed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("sequence space exhausted; re-key this source")]
pub struct SequenceExhausted;

impl SequenceCounter {
    /// Counter whose next value is 1.
    pub fn new() -> Self {
        Self::default()
    }

    /// Counter whose next value is `last + 1` (resume after restart).
    pub fn resume_after(last: u64) -> Self {
        Self { last }
    }

    /// Next sequence number.
    pub fn advance(&mut self) -> Result<u64, SequenceExhausted> {
        let n = self.last.checked_add(1).ok_or(SequenceExhausted)?;
        self.last = n;
        Ok(n)
    }

    /// Last value handed out (0 = none yet).
    pub fn last(&self) -> u64 {
        self.last
    }
}

/// Replay-window parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayConfig {
    /// Freshness bound Δmax for ordinary events, ms (spec: 2000).
    pub max_skew_ms: i64,
    /// Oldest acceptable `replayed` event, ms.
    pub replay_max_age_ms: i64,
    /// Accepted sequence numbers remembered per source, used only to
    /// distinguish duplicates from other out-of-order events.
    pub history: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_skew_ms: 2_000,
            replay_max_age_ms: 300_000,
            history: 1_024,
        }
    }
}

/// Why the window refused an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReplayRejection {
    /// `(source, seq)` was already accepted.
    #[error("duplicate seq {seq}")]
    Duplicate {
        /// Rejected seq.
        seq: u64,
    },
    /// `seq` is not greater than the last accepted one and was not seen.
    #[error("out-of-order seq {seq} <= last {last}")]
    OutOfOrder {
        /// Rejected seq.
        seq: u64,
        /// Last accepted seq.
        last: u64,
    },
    /// Ordinary event older than Δmax.
    #[error("stale: {age_ms} ms old (max {max_ms})")]
    Stale {
        /// Age.
        age_ms: i64,
        /// Bound.
        max_ms: i64,
    },
    /// Timestamp more than Δmax in the future.
    #[error("from the future: {ahead_ms} ms ahead (max {max_ms})")]
    FromFuture {
        /// Lead.
        ahead_ms: i64,
        /// Bound.
        max_ms: i64,
    },
    /// Buffered replay older than `replay_max_age`.
    #[error("replayed event too old: {age_ms} ms (max {max_ms})")]
    ReplayTooOld {
        /// Age.
        age_ms: i64,
        /// Bound.
        max_ms: i64,
    },
}

#[derive(Debug, Clone, Default)]
struct SourceState {
    last: u64,
    /// Recently accepted seqs, strictly increasing.
    recent: VecDeque<u64>,
}

/// Receiver-side replay window: O(sources × history) memory. Only sources
/// whose signature already verified reach the window, so the number of
/// sources is bounded by the key registry.
#[derive(Debug, Clone, Default)]
pub struct ReplayWindow {
    cfg: ReplayConfig,
    sources: BTreeMap<String, SourceState>,
}

/// How an accepted event was judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Admission {
    /// Fresh event within Δmax.
    Fresh,
    /// Buffered replay judged on sequence number (S7).
    Replayed,
}

impl ReplayWindow {
    /// New window.
    pub fn new(cfg: ReplayConfig) -> Self {
        Self {
            cfg,
            sources: BTreeMap::new(),
        }
    }

    /// Parameters in use.
    pub fn config(&self) -> ReplayConfig {
        self.cfg
    }

    /// Last accepted seq for `source` (0 = none).
    pub fn last(&self, source: &str) -> u64 {
        self.sources.get(source).map_or(0, |s| s.last)
    }

    /// Checks and, if accepted, records `(source, seq)`. Call only after the
    /// signature verified.
    pub fn admit(
        &mut self,
        source: &str,
        seq: u64,
        t_ms: i64,
        replayed: bool,
        now_ms: i64,
    ) -> Result<Admission, ReplayRejection> {
        let max = self.cfg.max_skew_ms;
        let ahead = t_ms.saturating_sub(now_ms);
        if ahead > max {
            return Err(ReplayRejection::FromFuture {
                ahead_ms: ahead,
                max_ms: max,
            });
        }
        let age = now_ms.saturating_sub(t_ms);
        let admission = if replayed {
            if age > self.cfg.replay_max_age_ms {
                return Err(ReplayRejection::ReplayTooOld {
                    age_ms: age,
                    max_ms: self.cfg.replay_max_age_ms,
                });
            }
            Admission::Replayed
        } else {
            if age > max {
                return Err(ReplayRejection::Stale {
                    age_ms: age,
                    max_ms: max,
                });
            }
            Admission::Fresh
        };

        let st = self.sources.entry(source.to_string()).or_default();
        if seq <= st.last {
            return Err(if st.recent.binary_search(&seq).is_ok() {
                ReplayRejection::Duplicate { seq }
            } else {
                ReplayRejection::OutOfOrder { seq, last: st.last }
            });
        }
        st.last = seq;
        st.recent.push_back(seq);
        while st.recent.len() > self.cfg.history {
            st.recent.pop_front();
        }
        Ok(admission)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_798_761_600_000;

    #[test]
    fn counter_starts_at_one_and_refuses_to_wrap() {
        let mut c = SequenceCounter::new();
        assert_eq!(c.advance(), Ok(1));
        assert_eq!(c.advance(), Ok(2));
        let mut c = SequenceCounter::resume_after(u64::MAX - 1);
        assert_eq!(c.advance(), Ok(u64::MAX));
        assert_eq!(c.advance(), Err(SequenceExhausted));
        assert_eq!(c.advance(), Err(SequenceExhausted));
        assert_eq!(c.last(), u64::MAX);
    }

    #[test]
    fn strictly_increasing_per_source() {
        let mut w = ReplayWindow::new(ReplayConfig::default());
        assert_eq!(w.admit("node-a", 1, NOW, false, NOW), Ok(Admission::Fresh));
        assert_eq!(w.admit("node-a", 2, NOW, false, NOW), Ok(Admission::Fresh));
        // Gaps are allowed (lost events are not replays).
        assert_eq!(w.admit("node-a", 10, NOW, false, NOW), Ok(Admission::Fresh));
        // Sources are independent.
        assert_eq!(w.admit("node-b", 1, NOW, false, NOW), Ok(Admission::Fresh));
        assert_eq!(
            w.admit("node-a", 10, NOW, false, NOW),
            Err(ReplayRejection::Duplicate { seq: 10 })
        );
        assert_eq!(
            w.admit("node-a", 2, NOW, false, NOW),
            Err(ReplayRejection::Duplicate { seq: 2 })
        );
        assert_eq!(
            w.admit("node-a", 5, NOW, false, NOW),
            Err(ReplayRejection::OutOfOrder { seq: 5, last: 10 })
        );
        assert_eq!(w.last("node-a"), 10);
    }

    #[test]
    fn freshness_boundary_is_inclusive_at_two_seconds() {
        let mut w = ReplayWindow::new(ReplayConfig::default());
        assert!(w.admit("s", 1, NOW - 2_000, false, NOW).is_ok());
        assert_eq!(
            w.admit("s", 2, NOW - 2_001, false, NOW),
            Err(ReplayRejection::Stale {
                age_ms: 2_001,
                max_ms: 2_000
            })
        );
        assert!(w.admit("s", 3, NOW + 2_000, false, NOW).is_ok());
        assert!(matches!(
            w.admit("s", 4, NOW + 2_001, false, NOW),
            Err(ReplayRejection::FromFuture { .. })
        ));
        // Rejected events do not advance the window.
        assert_eq!(w.last("s"), 3);
    }

    #[test]
    fn replayed_events_are_judged_on_sequence_not_freshness() {
        let mut w = ReplayWindow::new(ReplayConfig::default());
        // A 20 s partition buffer flushed in order.
        for (i, seq) in (100..300).enumerate() {
            let t = NOW - 20_000 + (i as i64) * 100;
            assert_eq!(
                w.admit("node-c", seq, t, true, NOW),
                Ok(Admission::Replayed)
            );
        }
        // Flushing the same buffer twice is caught.
        assert!(matches!(
            w.admit("node-c", 150, NOW - 15_000, true, NOW),
            Err(ReplayRejection::Duplicate { .. })
        ));
        // A replayed flag does not excuse a stale seq or an ancient timestamp.
        assert!(matches!(
            w.admit("node-c", 400, NOW - 300_001, true, NOW),
            Err(ReplayRejection::ReplayTooOld { .. })
        ));
        assert!(matches!(
            w.admit("node-c", 401, NOW + 5_000, true, NOW),
            Err(ReplayRejection::FromFuture { .. })
        ));
        // Live traffic continues after the flush.
        assert_eq!(
            w.admit("node-c", 300, NOW, false, NOW),
            Ok(Admission::Fresh)
        );
    }

    #[test]
    fn max_sequence_is_terminal_for_a_source() {
        let mut w = ReplayWindow::new(ReplayConfig::default());
        assert!(w.admit("s", u64::MAX, NOW, false, NOW).is_ok());
        assert!(w.admit("s", u64::MAX, NOW, false, NOW).is_err());
        assert!(w.admit("s", 1, NOW, false, NOW).is_err());
    }

    #[test]
    fn history_is_bounded() {
        let cfg = ReplayConfig {
            history: 8,
            ..ReplayConfig::default()
        };
        let mut w = ReplayWindow::new(cfg);
        for s in 1..=100 {
            w.admit("s", s, NOW, false, NOW).unwrap();
        }
        assert_eq!(w.sources["s"].recent.len(), 8);
        // Old duplicates beyond history are still rejected (as out-of-order).
        assert!(matches!(
            w.admit("s", 3, NOW, false, NOW),
            Err(ReplayRejection::OutOfOrder { .. })
        ));
    }

    #[test]
    fn extreme_timestamps_do_not_overflow() {
        let mut w = ReplayWindow::new(ReplayConfig::default());
        assert!(w.admit("s", 1, i64::MIN, false, i64::MAX).is_err());
        assert!(w.admit("s", 2, i64::MAX, false, i64::MIN).is_err());
        assert!(w.admit("s", 3, i64::MIN, true, i64::MAX).is_err());
    }
}
