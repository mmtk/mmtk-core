use super::layout::vm_layout_constants::*;
use crate::util::constants::*;
use crate::util::Address;

#[derive(Clone, Copy, Debug)]
pub enum VMRequest {
    Discontiguous,
    Fixed {
        start: Address,
        extent: usize,
        top: bool,
    },
    Extent {
        extent: usize,
        top: bool,
    },
    Fraction {
        frac: f32,
        top: bool,
    },
}

impl VMRequest {
    pub fn is_discontiguous(&self) -> bool {
        matches!(self, VMRequest::Discontiguous { .. })
    }

    pub fn common64bit(top: bool) -> Self {
        VMRequest::Extent {
            extent: VM_LAYOUT_CONSTANTS.max_space_extent(),
            top,
        }
    }

    pub fn discontiguous() -> Self {
        if cfg!(target_pointer_width = "64") && VM_LAYOUT_CONSTANTS.log_address_space > 35 {
            return Self::common64bit(false);
        }
        VMRequest::Discontiguous
    }

    pub fn fixed_size(mb: usize) -> Self {
        if cfg!(target_pointer_width = "64") && VM_LAYOUT_CONSTANTS.log_address_space > 35 {
            return Self::common64bit(false);
        }
        VMRequest::Extent {
            extent: mb << LOG_BYTES_IN_MBYTE,
            top: false,
        }
    }

    pub fn fraction(frac: f32) -> Self {
        if cfg!(target_pointer_width = "64") && VM_LAYOUT_CONSTANTS.log_address_space > 35 {
            return Self::common64bit(false);
        }
        VMRequest::Fraction { frac, top: false }
    }

    pub fn high_fixed_size(mb: usize) -> Self {
        if cfg!(target_pointer_width = "64") && VM_LAYOUT_CONSTANTS.log_address_space > 35 {
            return Self::common64bit(true);
        }
        VMRequest::Extent {
            extent: mb << LOG_BYTES_IN_MBYTE,
            top: true,
        }
    }

    pub fn fixed_extent(extent: usize, top: bool) -> Self {
        if cfg!(target_pointer_width = "64") && VM_LAYOUT_CONSTANTS.log_address_space > 35 {
            return Self::common64bit(top);
        }
        VMRequest::Extent { extent, top }
    }
}
