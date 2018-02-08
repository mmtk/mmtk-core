use ::util::Address;
use ::util::constants::*;
use super::layout::heap_parameters::*;
//use super::layout::vm_layout_constants::*;

////////// FIXME //////////////
#[cfg(target_pointer_width = "32")]
const HEAP_LAYOUT_32BIT: bool = false;
#[cfg(target_pointer_width = "64")]
const HEAP_LAYOUT_32BIT: bool = false;
const HEAP_LAYOUT_64BIT: bool = !HEAP_LAYOUT_32BIT;
const LOG_SPACE_EXTENT: usize = if_then_else_usize!(HEAP_LAYOUT_64BIT, LOG_SPACE_SIZE_64, 31);
const MAX_SPACE_EXTENT: usize = 1 << LOG_SPACE_EXTENT;

pub enum VMRequest {
    RequestDiscontiguous,
    RequestFixed, // XXX: Never used?
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
        if HEAP_LAYOUT_64BIT {
            Self::common64bit(false)
        } else {
            VMRequest::RequestDiscontiguous
        }
    }

    pub fn fixed_size(mb: usize) -> Self {
        if HEAP_LAYOUT_64BIT {
            Self::common64bit(false)
        } else {
            VMRequest::RequestExtent {
                extent: mb << LOG_BYTES_IN_MBYTE,
                top: false,
            }
        }
    }

    pub fn fraction(frac: f32) -> Self {
        if HEAP_LAYOUT_64BIT {
            Self::common64bit(false)
        } else {
            VMRequest::RequestFraction {
                frac,
                top: false,
            }
        }
    }

    pub fn high_fixed_size(mb: usize) -> Self {
        if HEAP_LAYOUT_64BIT {
            Self::common64bit(false)
        } else {
            VMRequest::RequestExtent {
                extent: mb << LOG_BYTES_IN_MBYTE,
                top: true,
            }
        }
    }

    pub fn fixed_extent(extent: usize, top: bool) -> Self {
        if HEAP_LAYOUT_64BIT {
            Self::common64bit(false)
        } else {
            VMRequest::RequestExtent {
                extent,
                top,
            }
        }
    }
}

