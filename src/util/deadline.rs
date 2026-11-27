#![allow(dead_code)]
//! A cheap, shared deadline for time-bounded GC work, such as the
//! increments of an incremental mark.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

lazy_static! {
    static ref EPOCH: Instant = Instant::now();
}

/// Nanoseconds since the process-wide epoch.
pub(crate) fn now_nanos() -> u64 {
    EPOCH.elapsed().as_nanos() as u64
}

pub(crate) struct DeadlineClock {
    deadline_nanos: AtomicU64,
}

impl Default for DeadlineClock {
    fn default() -> Self {
        Self {
            deadline_nanos: AtomicU64::new(u64::MAX),
        }
    }
}

impl DeadlineClock {
    /// Set the deadline `budget` from now.
    pub fn arm(&self, budget: Duration) {
        let d = now_nanos().saturating_add(budget.as_nanos() as u64);
        self.deadline_nanos.store(d, Ordering::SeqCst);
    }

    /// Remove the deadline: work runs to completion.
    pub fn disarm(&self) {
        self.deadline_nanos.store(u64::MAX, Ordering::SeqCst);
    }

    /// Force all deadline checks to report "expired", e.g. when a pause must end early.
    pub fn expire_now(&self) {
        self.deadline_nanos.store(0, Ordering::SeqCst);
    }

    /// Return `true` if the deadline has passed.
    pub fn expired(&self) -> bool {
        let d = self.deadline_nanos.load(Ordering::Relaxed);
        d != u64::MAX && now_nanos() >= d
    }

    /// Return `true` if a deadline is set.
    pub fn is_armed(&self) -> bool {
        self.deadline_nanos.load(Ordering::Relaxed) != u64::MAX
    }
}
