//! Hybrid logical clock (§6.2).
//!
//! ```text
//! send/local:  l' = max(l, pt),        c' = c+1 if l' = l else 0
//! receive m:   l' = max(l, l_m, pt),   c' = max(c, c_m)+1 if l' = l = l_m
//!                                           c+1            if l' = l ≠ l_m
//!                                           c_m+1          if l' = l_m ≠ l
//!                                           0              otherwise
//! ```
//!
//! Two guards the formulas leave implicit:
//! * the counter is a `u32`; overflow is an error, never a wrap;
//! * a remote `l_m` more than `max_forward_ms` ahead of local physical time
//!   is rejected, otherwise one peer with a bad (or malicious) clock could
//!   drag every clock in the system into the future.

use aeris_types::Hlc;

/// HLC failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HlcError {
    /// The logical counter would exceed `u32::MAX`.
    #[error("HLC counter overflow at l={l}")]
    CounterOverflow {
        /// Physical component at the time of overflow.
        l: i64,
    },
    /// A remote timestamp is too far ahead of local physical time.
    #[error("remote HLC l={remote_l} is {ahead_ms} ms ahead of local physical time (max {max_ms})")]
    RemoteTooFarAhead {
        /// Remote physical component.
        remote_l: i64,
        /// How far ahead it is.
        ahead_ms: i64,
        /// Allowed maximum.
        max_ms: i64,
    },
}

/// A node's hybrid logical clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HlcClock {
    l: i64,
    c: u32,
    max_forward_ms: i64,
}

impl HlcClock {
    /// New clock at `(0, 0)`. `max_forward_ms` bounds how far a remote
    /// timestamp may be ahead of local physical time.
    pub fn new(max_forward_ms: i64) -> Self {
        Self {
            l: 0,
            c: 0,
            max_forward_ms,
        }
    }

    /// Current value without advancing.
    pub fn now(&self) -> Hlc {
        Hlc {
            l: self.l,
            c: self.c,
        }
    }

    /// Send or local event at physical time `pt` (ms).
    pub fn send(&mut self, pt: i64) -> Result<Hlc, HlcError> {
        let l_new = self.l.max(pt);
        let c_new = if l_new == self.l {
            self.c
                .checked_add(1)
                .ok_or(HlcError::CounterOverflow { l: l_new })?
        } else {
            0
        };
        self.l = l_new;
        self.c = c_new;
        Ok(self.now())
    }

    /// Receive a message stamped `m` at local physical time `pt`.
    pub fn receive(&mut self, m: Hlc, pt: i64) -> Result<Hlc, HlcError> {
        let ahead = m.l.saturating_sub(pt);
        if ahead > self.max_forward_ms {
            return Err(HlcError::RemoteTooFarAhead {
                remote_l: m.l,
                ahead_ms: ahead,
                max_ms: self.max_forward_ms,
            });
        }
        let l_new = self.l.max(m.l).max(pt);
        let inc = |x: u32| {
            x.checked_add(1)
                .ok_or(HlcError::CounterOverflow { l: l_new })
        };
        let c_new = if l_new == self.l && l_new == m.l {
            inc(self.c.max(m.c))?
        } else if l_new == self.l {
            inc(self.c)?
        } else if l_new == m.l {
            inc(m.c)?
        } else {
            0
        };
        self.l = l_new;
        self.c = c_new;
        Ok(self.now())
    }
}

/// Total order used by the server: `(l, c, source)`.
pub fn hlc_order_key<'a>(hlc: &Hlc, source: &'a str) -> (i64, u32, &'a str) {
    (hlc.l, hlc.c, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_advances_with_physical_time() {
        let mut c = HlcClock::new(2_000);
        assert_eq!(c.send(100).unwrap(), Hlc { l: 100, c: 0 });
        // Same physical time: counter increments.
        assert_eq!(c.send(100).unwrap(), Hlc { l: 100, c: 1 });
        // Physical clock went backwards: l stays, counter increments.
        assert_eq!(c.send(90).unwrap(), Hlc { l: 100, c: 2 });
        assert_eq!(c.send(101).unwrap(), Hlc { l: 101, c: 0 });
    }

    #[test]
    fn receive_follows_all_four_cases() {
        // l' = l = l_m
        let mut c = HlcClock::new(2_000);
        c.send(100).unwrap();
        c.send(100).unwrap(); // (100,1)
        assert_eq!(
            c.receive(Hlc { l: 100, c: 5 }, 50).unwrap(),
            Hlc { l: 100, c: 6 }
        );
        // l' = l != l_m
        assert_eq!(
            c.receive(Hlc { l: 80, c: 9 }, 50).unwrap(),
            Hlc { l: 100, c: 7 }
        );
        // l' = l_m != l
        assert_eq!(
            c.receive(Hlc { l: 150, c: 3 }, 120).unwrap(),
            Hlc { l: 150, c: 4 }
        );
        // physical time dominates
        assert_eq!(
            c.receive(Hlc { l: 140, c: 3 }, 200).unwrap(),
            Hlc { l: 200, c: 0 }
        );
    }

    #[test]
    fn counter_overflow_is_an_error_not_a_wrap() {
        let mut c = HlcClock {
            l: 10,
            c: u32::MAX,
            max_forward_ms: 2_000,
        };
        assert_eq!(c.send(10), Err(HlcError::CounterOverflow { l: 10 }));
        // State is unchanged after the error.
        assert_eq!(c.now(), Hlc { l: 10, c: u32::MAX });
        // Physical time moving forward resets the counter.
        assert_eq!(c.send(11).unwrap(), Hlc { l: 11, c: 0 });
    }

    #[test]
    fn rejects_remote_far_in_future() {
        let mut c = HlcClock::new(2_000);
        let err = c.receive(Hlc { l: 1_000_000, c: 0 }, 1_000).unwrap_err();
        assert!(matches!(err, HlcError::RemoteTooFarAhead { .. }));
        assert_eq!(c.now(), Hlc { l: 0, c: 0 });
    }

    #[test]
    fn hlc_is_monotonic_under_random_inputs() {
        // Deterministic pseudo-random walk; every output must be > previous.
        let mut c = HlcClock::new(10_000);
        let mut prev = c.now();
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..10_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let pt = 1_000 + (x % 500) as i64;
            let out = if x & 1 == 0 {
                c.send(pt).unwrap()
            } else {
                let m = Hlc {
                    l: 1_000 + ((x >> 8) % 600) as i64,
                    c: ((x >> 20) % 10) as u32,
                };
                c.receive(m, pt).unwrap()
            };
            assert!((out.l, out.c) > (prev.l, prev.c), "{out:?} !> {prev:?}");
            prev = out;
        }
    }
}
