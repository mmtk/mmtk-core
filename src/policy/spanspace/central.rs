//! Per-span-class central span sets (Go's `mcentral`, `runtime/mcentral.go`).
//!
//! Each span class has `partial` and `full` sets, each split into a swept and an unswept half.
//! The halves trade roles every GC cycle: `sweepgen` grows by 2 per cycle, so the swept spans are
//! in `partial[sweepgen/2%2]` and the unswept ones in `partial[1-sweepgen/2%2]`. Spans pushed as
//! "swept" in one cycle are therefore "unswept" in the next without being moved.
//!
//! Go's sets are lock-free `spanSet`s (`runtime/mspanset.go`). These are mutex-protected vectors
//! (deviation SP3, decision D32).

use super::classes::SpanClass;
use super::classes::NUM_SPAN_CLASSES;
use super::span::SpanId;
use std::sync::Mutex;

#[derive(Default)]
struct ClassSets {
    partial: [Vec<SpanId>; 2],
    full: [Vec<SpanId>; 2],
}

/// The central span sets of all span classes.
pub struct Central {
    classes: Vec<Mutex<ClassSets>>,
}

impl Default for Central {
    fn default() -> Self {
        Self::new()
    }
}

/// Go: `partialSwept`/`fullSwept` index.
fn swept(sweepgen: u32) -> usize {
    (sweepgen / 2 % 2) as usize
}

/// Go: `partialUnswept`/`fullUnswept` index.
fn unswept(sweepgen: u32) -> usize {
    1 - swept(sweepgen)
}

impl Central {
    pub fn new() -> Self {
        Central {
            classes: (0..NUM_SPAN_CLASSES)
                .map(|_| Mutex::new(ClassSets::default()))
                .collect(),
        }
    }

    pub fn push_partial_swept(&self, spc: SpanClass, sg: u32, id: SpanId) {
        self.classes[spc.index()].lock().unwrap().partial[swept(sg)].push(id);
    }

    pub fn push_full_swept(&self, spc: SpanClass, sg: u32, id: SpanId) {
        self.classes[spc.index()].lock().unwrap().full[swept(sg)].push(id);
    }

    pub fn pop_partial_swept(&self, spc: SpanClass, sg: u32) -> Option<SpanId> {
        self.classes[spc.index()].lock().unwrap().partial[swept(sg)].pop()
    }

    pub fn pop_partial_unswept(&self, spc: SpanClass, sg: u32) -> Option<SpanId> {
        self.classes[spc.index()].lock().unwrap().partial[unswept(sg)].pop()
    }

    pub fn pop_full_unswept(&self, spc: SpanClass, sg: u32) -> Option<SpanId> {
        self.classes[spc.index()].lock().unwrap().full[unswept(sg)].pop()
    }

    /// Pop any unswept span of class `spc`, partial first (the order of Go's
    /// `nextSpanForSweep`).
    pub fn pop_unswept(&self, spc: SpanClass, sg: u32) -> Option<SpanId> {
        let mut sets = self.classes[spc.index()].lock().unwrap();
        let i = unswept(sg);
        sets.partial[i].pop().or_else(|| sets.full[i].pop())
    }

    /// Number of spans in the unswept sets of all classes.
    pub fn count_unswept(&self, sg: u32) -> usize {
        let i = unswept(sg);
        self.classes
            .iter()
            .map(|c| {
                let c = c.lock().unwrap();
                c.partial[i].len() + c.full[i].len()
            })
            .sum()
    }

    /// Number of spans in the swept sets of all classes.
    pub fn count_swept(&self, sg: u32) -> usize {
        let i = swept(sg);
        self.classes
            .iter()
            .map(|c| {
                let c = c.lock().unwrap();
                c.partial[i].len() + c.full[i].len()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spans pushed as swept in one cycle are unswept after `sweepgen += 2`.
    #[test]
    fn swept_and_unswept_trade_roles() {
        let c = Central::new();
        let spc = SpanClass::new(3, false);
        let mut sg = 4;
        c.push_partial_swept(spc, sg, 7);
        c.push_full_swept(spc, sg, 8);
        assert_eq!(c.count_swept(sg), 2);
        assert_eq!(c.count_unswept(sg), 0);
        sg += 2;
        assert_eq!(c.count_swept(sg), 0);
        assert_eq!(c.count_unswept(sg), 2);
        assert_eq!(c.pop_partial_swept(spc, sg), None);
        assert_eq!(c.pop_unswept(spc, sg), Some(7));
        assert_eq!(c.pop_full_unswept(spc, sg), Some(8));
        assert_eq!(c.pop_unswept(spc, sg), None);
        // Other classes are separate.
        c.push_partial_swept(SpanClass::new(3, true), sg, 9);
        assert_eq!(c.pop_partial_swept(spc, sg), None);
        assert_eq!(c.pop_partial_swept(SpanClass::new(3, true), sg), Some(9));
    }
}
