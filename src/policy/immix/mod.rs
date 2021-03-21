pub mod immixspace;
pub mod block;
pub mod line;
pub mod chunk;
pub mod defrag;

pub use immixspace::*;

/// Mark/sweep memory for block-level only
pub const BLOCK_ONLY: bool = false;

/// Opportunistic copying
pub const DEFRAG: bool = true;


macro_rules! validate {
    ($x: expr) => { assert!($x, stringify!($x)) };
    ($x: expr => $y: expr) => { if $x { assert!($y, stringify!($x implies $y)) } };
}

const fn validate_features() {
    // validate!(LINE_COUNTER => !BLOCK_ONLY);
    validate!(DEFRAG => !BLOCK_ONLY);
}