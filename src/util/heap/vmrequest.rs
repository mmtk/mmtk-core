use super::layout::vm_layout::*;
use crate::util::constants::*;
use crate::util::Address;

#[derive(Clone, Copy, Debug)]
pub enum VMRequest {
    Discontiguous,
    Fixed { start: Address, extent: usize },
    Extent { extent: usize, top: bool },
    Fraction { frac: f32, top: bool },
}

impl VMRequest {
    pub fn is_discontiguous(&self) -> bool {
        matches!(self, VMRequest::Discontiguous { .. })
    }

    pub fn common64bit(top: bool) -> Self {
        VMRequest::Extent {
            extent: vm_layout().max_space_extent(),
            top,
        }
    }

    pub fn discontiguous() -> Self {
        if cfg!(target_pointer_width = "64") && vm_layout().force_use_contiguous_spaces {
            return Self::common64bit(false);
        }
        VMRequest::Discontiguous
    }

    pub fn fixed_size(mb: usize) -> Self {
        if cfg!(target_pointer_width = "64") && vm_layout().force_use_contiguous_spaces {
            return Self::common64bit(false);
        }
        VMRequest::Extent {
            extent: mb << LOG_BYTES_IN_MBYTE,
            top: false,
        }
    }

    pub fn fraction(frac: f32) -> Self {
        if cfg!(target_pointer_width = "64") && vm_layout().force_use_contiguous_spaces {
            return Self::common64bit(false);
        }
        VMRequest::Fraction { frac, top: false }
    }

    pub fn high_fixed_size(mb: usize) -> Self {
        if cfg!(target_pointer_width = "64") && vm_layout().force_use_contiguous_spaces {
            return Self::common64bit(true);
        }
        VMRequest::Extent {
            extent: mb << LOG_BYTES_IN_MBYTE,
            top: true,
        }
    }

    pub fn fixed_extent(extent: usize, top: bool) -> Self {
        if cfg!(target_pointer_width = "64") && vm_layout().force_use_contiguous_spaces {
            return Self::common64bit(top);
        }
        VMRequest::Extent { extent, top }
    }

    pub fn fixed(start: Address, extent: usize) -> Self {
        VMRequest::Fixed { start, extent }
    }
}
