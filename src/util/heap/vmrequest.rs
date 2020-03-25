use ::util::Address;
use ::util::constants::*;
use super::layout::vm_layout_constants::*;

////////// FIXME //////////////
#[cfg(target_pointer_width = "32")]
pub const HEAP_LAYOUT_32BIT: bool = true;
#[cfg(target_pointer_width = "64")]
pub const HEAP_LAYOUT_32BIT: bool = false; // FIXME SERIOUSLY
pub const HEAP_LAYOUT_64BIT: bool = !HEAP_LAYOUT_32BIT;

#[derive(Clone, Copy, Debug)]
pub enum VMRequest {
    RequestDiscontiguous,
    RequestFixed {
        start: Address,
        extent: usize,
        top: bool,
    },
    RequestExtent {
        extent: usize,
        top: bool,
    },
    RequestFraction {
        frac: f32,
        top: bool,
    },
}

impl VMRequest {
    pub fn is_discontiguous(&self) -> bool {
        match self {
            &VMRequest::RequestDiscontiguous{..} => true,
            _ => false,
        }
    }

    pub fn common64bit(top: bool) -> Self {
        VMRequest::RequestExtent {
            extent: MAX_SPACE_EXTENT,
            top,
        }
    }

    pub fn discontiguous() -> Self {
        VMRequest::RequestDiscontiguous
    }

    pub fn fixed_size(mb: usize) -> Self {
        VMRequest::RequestExtent {
            extent: mb << LOG_BYTES_IN_MBYTE,
            top: false,
        }
    }

    pub fn fraction(frac: f32) -> Self {
        VMRequest::RequestFraction {
            frac,
            top: false,
        }
    }

    pub fn high_fixed_size(mb: usize) -> Self {
        VMRequest::RequestExtent {
            extent: mb << LOG_BYTES_IN_MBYTE,
            top: true,
        }
    }

    pub fn fixed_extent(extent: usize, top: bool) -> Self {
        VMRequest::RequestExtent {
            extent,
            top,
        }
    }
}

