pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::{get_active_barrier, Immix};

pub const CONCURRENT_MARKING: bool = false;
