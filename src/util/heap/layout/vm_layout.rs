use super::heap_parameters::*;
use crate::util::constants::*;
use crate::util::Address;

use crate::util::conversions::{chunk_align_down, chunk_align_up};

/**
 * log_2 of the coarsest unit of address space allocation.
 * <p>
 * In the 32-bit VM layout, this determines the granularity of
 * allocation in a discontigouous space.  In the 64-bit layout,
 * this determines the growth factor of the large contiguous spaces
 * that we provide.
 */
pub const LOG_BYTES_IN_CHUNK: usize = 22;

/** Coarsest unit of address space allocation. */
pub const BYTES_IN_CHUNK: usize = 1 << LOG_BYTES_IN_CHUNK;
pub const CHUNK_MASK: usize = (1 << LOG_BYTES_IN_CHUNK) - 1;

/** Coarsest unit of address space allocation, in pages */
pub const PAGES_IN_CHUNK: usize = 1 << (LOG_BYTES_IN_CHUNK - LOG_BYTES_IN_PAGE as usize);

/** Granularity at which we map and unmap virtual address space in the heap */
pub const LOG_MMAP_CHUNK_BYTES: usize = LOG_BYTES_IN_CHUNK;

pub const MMAP_CHUNK_BYTES: usize = 1 << LOG_MMAP_CHUNK_BYTES;

/** log_2 of the number of pages in a 64-bit space */
pub const LOG_PAGES_IN_SPACE64: usize = LOG_SPACE_SIZE_64 - LOG_BYTES_IN_PAGE as usize;

/** The number of pages in a 64-bit space */
pub const PAGES_IN_SPACE64: usize = 1 << LOG_PAGES_IN_SPACE64;

/// Runtime-initialized virtual memory constants
#[derive(Clone)]
pub struct VMLayout {
    /// log_2 of the addressable heap virtual space.
    pub log_address_space: usize,
    /// FIXME: HEAP_START, HEAP_END are VM-dependent
    /// Lowest virtual address used by the virtual machine
    pub heap_start: Address,
    /// Highest virtual address used by the virtual machine
    pub heap_end: Address,
    /// An upper bound on the extent of any space in the
    /// current memory layout
    pub log_space_extent: usize,
    /// Should mmtk enable contiguous spaces and virtual memory for all spaces?
    /// For normal 64-bit config, this should be set to true. Each space should own a contiguous piece of virtual memory.
    /// For 32-bit or 64-bit compressed heap, we don't have enough virtual memory, so this should be set to false.
    pub force_use_contiguous_spaces: bool,
}

impl VMLayout {
    #[cfg(target_pointer_width = "32")]
    pub const LOG_ARCH_ADDRESS_SPACE: usize = 32;
    #[cfg(target_pointer_width = "64")]
    pub const LOG_ARCH_ADDRESS_SPACE: usize = 47;
    /// An upper bound on the extent of any space in the
    /// current memory layout
    pub const fn max_space_extent(&self) -> usize {
        1 << self.log_space_extent
    }
    /// Lowest virtual address available for MMTk to manage.
    pub const fn available_start(&self) -> Address {
        self.heap_start
    }
    /// Highest virtual address available for MMTk to manage.
    pub const fn available_end(&self) -> Address {
        self.heap_end
    }
    /// Size of the address space available to the MMTk heap.
    pub const fn available_bytes(&self) -> usize {
        self.available_end().get_extent(self.available_start())
    }
    /// Maximum number of chunks we need to track.  Only used in 32-bit layout.
    pub const fn max_chunks(&self) -> usize {
        1 << self.log_max_chunks()
    }
    /// log_2 of the maximum number of chunks we need to track.  Only used in 32-bit layout.
    pub const fn log_max_chunks(&self) -> usize {
        Self::LOG_ARCH_ADDRESS_SPACE - LOG_BYTES_IN_CHUNK
    }
    /// Number of bits to shift a space index into/out of a virtual address.
    /// In a 32-bit model, use a dummy value so that the compiler doesn't barf.
    pub(crate) fn space_shift_64(&self) -> usize {
        self.log_space_extent
    }
    /// Bitwise mask to isolate a space index in a virtual address.
    /// We can't express this constant in a 32-bit environment, hence the
    /// conditional definition.
    pub(crate) fn space_mask_64(&self) -> usize {
        ((1 << LOG_MAX_SPACES) - 1) << self.space_shift_64()
    }
    /// Size of each space in the 64-bit memory layout
    /// We can't express this constant in a 32-bit environment, hence the
    /// conditional definition.
    /// FIXME: When Compiling for 32 bits this expression makes no sense
    pub(crate) fn space_size_64(&self) -> usize {
        self.max_space_extent()
    }
}

impl VMLayout {
    /// Normal 32-bit configuration
    pub const fn new_32bit() -> Self {
        Self {
            log_address_space: 32,
            heap_start: chunk_align_down(unsafe { Address::from_usize(0x8000_0000) }),
            heap_end: chunk_align_up(unsafe { Address::from_usize(0xd000_0000) }),
            log_space_extent: 31,
            force_use_contiguous_spaces: false,
        }
    }
    /// Normal 64-bit configuration
    #[cfg(target_pointer_width = "32")]
    pub const fn new_64bit() -> Self {
        unimplemented!("64-bit heap constants do not work with 32-bit builds")
    }
    #[cfg(target_pointer_width = "64")]
    pub const fn new_64bit() -> Self {
        Self {
            log_address_space: 47,
            heap_start: chunk_align_down(unsafe {
                Address::from_usize(0x0000_0200_0000_0000usize)
            }),
            heap_end: chunk_align_up(unsafe { Address::from_usize(0x0000_2200_0000_0000usize) }),
            log_space_extent: 41,
            force_use_contiguous_spaces: true,
        }
    }

    /// Custom VM layout constants. VM bindings may use this function for compressed or 39-bit heap support.
    /// This function must be called before MMTk::new()
    pub fn set_custom_vm_layout(constants: VMLayout) {
        unsafe {
            VM_LAYOUT = constants;
        }
    }
}

#[cfg(target_pointer_width = "32")]
static mut VM_LAYOUT: VMLayout = VMLayout::new_32bit();
#[cfg(target_pointer_width = "64")]
static mut VM_LAYOUT: VMLayout = VMLayout::new_64bit();

pub fn vm_layout() -> &'static VMLayout {
    unsafe { &VM_LAYOUT }
}
