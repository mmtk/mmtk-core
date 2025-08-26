pub mod barrier;
pub(super) mod concurrent_marking_work;
pub(super) mod global;

pub mod immix;

use bytemuck::NoUninit;

/// The pause type for a concurrent GC phase.
// TODO: This is probably not be general enough for all the concurrent plans.
// TODO: We could consider moving this to specific plans later.
#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Copy, Clone, NoUninit)]
pub enum Pause {
    Full = 1,
    InitialMark,
    FinalMark,
}

unsafe impl bytemuck::ZeroableInOption for Pause {}

unsafe impl bytemuck::PodInOption for Pause {}

impl Default for Pause {
    fn default() -> Self {
        Self::Full
    }
}
