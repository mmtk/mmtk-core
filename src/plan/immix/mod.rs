pub(super) mod gc_work;
pub(super) mod global;
pub(super) mod mutator;

pub use self::global::Immix;
pub use self::global::IMMIX_CONSTRAINTS;

use bytemuck::NoUninit;

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Copy, Clone, NoUninit)]
pub enum Pause {
    Full = 1,
    FullDefrag,
    RefCount,
    InitialMark,
    FinalMark,
}

unsafe impl bytemuck::ZeroableInOption for Pause {}
unsafe impl bytemuck::PodInOption for Pause {}
