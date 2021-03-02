pub mod immixspace;
pub mod block;
pub mod line;

pub use immixspace::*;

/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = false;

/// Use (sloppy) line counter as block mark
pub const LINE_COUNTER: bool = true;



macro_rules! validate {
    ($x: expr) => { assert!($x, stringify!($x)) };
    ($x: expr => $y: expr) => { if $x { assert!($y, stringify!($x implies $y)) } };
}

const fn validate_features() {
    validate!(LINE_COUNTER => !BLOCK_ONLY);
}