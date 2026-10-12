//! `SpanSpace`: a port of Go's span heap (design: `docs/design/go-gc-port.md` §3.5, §5.3-§5.5).
//!
//! This module currently holds Go's size classes ([`sizeclasses`], generated from Go's own
//! generator) and the lookups on them ([`classes`]).

// The size-class tables are used by the space and allocator added later.
#![allow(dead_code)]

pub mod classes;
#[rustfmt::skip]
#[allow(clippy::all, dead_code)]
pub mod sizeclasses;

#[allow(unused_imports)]
pub use classes::{SpanClass, BYTES_IN_SPAN_PAGE, LOG_BYTES_IN_SPAN_PAGE, NUM_SPAN_CLASSES};
