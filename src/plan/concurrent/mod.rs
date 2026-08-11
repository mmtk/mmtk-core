pub mod barrier;
pub(super) mod concurrent_marking_work;
pub mod global;

pub use self::global::ConcurrentPlan;

pub mod immix;

use bytemuck::NoUninit;

/// The pause type for a concurrent GC phase.
// TODO: This is probably not be general enough for all the concurrent plans.
// TODO: We could consider moving this to specific plans later.
#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Copy, Clone, NoUninit, Default)]
pub enum Pause {
    /// A whole GC (including root scanning, closure, releasing, etc.) happening in a single pause.
    ///
    /// Don't be confused with "full-heap" GC in generational collectors.  `Pause::Full` can also
    /// refer to a nursery GC that happens in a single pause.
    #[default]
    Full = 1,
    /// The initial pause before concurrent marking.
    InitialMark,
    /// The pause after concurrent marking.
    FinalMark,
    /// A reference-counting-only pause. Used by plans that combine reference counting with
    /// optional concurrent marking (e.g. LXR). Concurrent marking is NOT active during this
    /// pause — any in-flight concurrent work is postponed.
    RefCount,
}

unsafe impl bytemuck::ZeroableInOption for Pause {}

unsafe impl bytemuck::PodInOption for Pause {}
