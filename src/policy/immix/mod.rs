pub mod immixspace;
pub mod block;
pub mod line;
pub mod chunk;

pub use immixspace::*;

/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = false;

/// Use (sloppy) line counter as block mark
// pub const LINE_COUNTER: bool = true;

/// Opportunistic copying
pub const DEFRAG: bool = false;
/// A block will be marked as defrag if the number of marked lines is less than this threshold
pub const DEFRAG_THRESHOLD: usize = 16;


macro_rules! validate {
    ($x: expr) => { assert!($x, stringify!($x)) };
    ($x: expr => $y: expr) => { if $x { assert!($y, stringify!($x implies $y)) } };
}

const fn validate_features() {
    // validate!(LINE_COUNTER => !BLOCK_ONLY);
    validate!(DEFRAG => !BLOCK_ONLY);
}