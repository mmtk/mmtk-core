pub mod barrier;
pub mod concurrent_marking_work;
pub mod immix;

use bytemuck::NoUninit;

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
