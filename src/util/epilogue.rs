//! Utilities for implementing epilogues.
//!
//! Epilogues are operations that are done when several other operations are done.

use std::sync::atomic::{AtomicUsize, Ordering};

/// A debugging method for detecting the case when the epilogue is never executed.
pub fn debug_assert_counter_zero(counter: &AtomicUsize, what: &'static str) {
    let value = counter.load(Ordering::SeqCst);
    if value != 0 {
        panic!("{what} is still {value}.");
    }
}
