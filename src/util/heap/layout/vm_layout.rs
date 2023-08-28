use std::sync::atomic::AtomicBool;

use atomic::Ordering;

use super::heap_parameters::*;
use crate::util::constants::*;
use crate::util::Address;

use crate::util::conversions::{chunk_align_down, chunk_align_up};

/**
 * log_2 of the coarsest unit of address space allocation.
 *
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

/// Runtime-initialized virtual memory constants
#[derive(Clone, Debug)]
pub struct VMLayout {
    /// log_2 of the addressable heap virtual space.
    pub log_address_space: usize,
    /// Lowest virtual address used by the virtual machine. Should be chunk aligned.
    pub heap_start: Address,
    /// Highest virtual address used by the virtual machine. Should be chunk aligned.
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
    /// log_2 of the number of pages in a 64-bit space
    pub(crate) fn log_pages_in_space64(&self) -> usize {
        self.log_space_extent - LOG_BYTES_IN_PAGE as usize
    }
    /// The number of pages in a 64-bit space
    pub(crate) fn pages_in_space64(&self) -> usize {
        1 << self.log_pages_in_space64()
    }

    /// This mask extracts a few bits from address, and use it as index to the space map table.
    /// When masked with this constant, the index is 1 to 16. If we mask any arbitrary address with this mask, we will get 0 to 31 (32 entries).
    pub(crate) fn address_mask(&self) -> usize {
        0x1f << self.log_space_extent
    }

    const fn validate(&self) {
        assert!(self.heap_start.is_aligned_to(BYTES_IN_CHUNK));
        assert!(self.heap_end.is_aligned_to(BYTES_IN_CHUNK));
        assert!(self.heap_start.as_usize() < self.heap_end.as_usize());
        assert!(self.log_address_space <= Self::LOG_ARCH_ADDRESS_SPACE);
        assert!(self.log_space_extent <= self.log_address_space);
        if self.force_use_contiguous_spaces {
            assert!(self.log_space_extent <= (self.log_address_space - LOG_MAX_SPACES));
            assert!(self.heap_start.is_aligned_to(self.max_space_extent()));
        }
    }
}

impl VMLayout {
    /// Normal 32-bit configuration
    pub const fn new_32bit() -> Self {
        let layout32 = Self {
            log_address_space: 32,
            heap_start: chunk_align_down(unsafe { Address::from_usize(0x8000_0000) }),
            heap_end: chunk_align_up(unsafe { Address::from_usize(0xd000_0000) }),
            log_space_extent: 31,
            force_use_contiguous_spaces: false,
        };
        layout32.validate();
        layout32
    }
    /// Normal 64-bit configuration
    #[cfg(target_pointer_width = "64")]
    pub const fn new_64bit() -> Self {
        let layout64 = Self {
            log_address_space: 47,
            heap_start: chunk_align_down(unsafe {
                Address::from_usize(0x0000_0200_0000_0000usize)
            }),
            heap_end: chunk_align_up(unsafe { Address::from_usize(0x0000_2200_0000_0000usize) }),
            log_space_extent: 41,
            force_use_contiguous_spaces: true,
        };
        layout64.validate();
        layout64
    }

    /// Custom VM layout constants. VM bindings may use this function for compressed or 39-bit heap support.
    /// This function must be called before MMTk::new()
    pub(crate) fn set_custom_vm_layout(constants: VMLayout) {
        if cfg!(debug_assertions) {
            assert!(
                !VM_LAYOUT_FETCHED.load(Ordering::SeqCst),
                "vm_layout is already been used before setup"
            );
        }
        constants.validate();
        unsafe {
            VM_LAYOUT = constants;
        }
    }
}

// Implement default so bindings can selectively change some parameters while using default for others.
impl std::default::Default for VMLayout {
    #[cfg(target_pointer_width = "32")]
    fn default() -> Self {
        Self::new_32bit()
    }

    #[cfg(target_pointer_width = "64")]
    fn default() -> Self {
        Self::new_64bit()
    }
}

#[cfg(target_pointer_width = "32")]
static mut VM_LAYOUT: VMLayout = VMLayout::new_32bit();
#[cfg(target_pointer_width = "64")]
static mut VM_LAYOUT: VMLayout = VMLayout::new_64bit();

static VM_LAYOUT_FETCHED: AtomicBool = AtomicBool::new(false);

pub fn vm_layout() -> &'static VMLayout {
    if cfg!(debug_assertions) {
        VM_LAYOUT_FETCHED.store(true, Ordering::SeqCst);
    }
    unsafe { &VM_LAYOUT }
}
