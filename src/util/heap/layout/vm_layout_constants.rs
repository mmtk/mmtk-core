use spin::Mutex;

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
pub struct VMLayoutConstants {
    /// log_2 of the addressable heap virtual space.
    pub log_address_space: usize,
    /// FIXME: HEAP_START, HEAP_END are VM-dependent
    /// Lowest virtual address used by the virtual machine
    pub heap_start: Address,
    /// Highest virtual address used by the virtual machine
    pub heap_end: Address,
    /// log_2 of the maximum number of chunks we need to track.  Only used in 32-bit layout.
    pub log_max_chunks: usize,
    /// An upper bound on the extent of any space in the
    /// current memory layout
    pub log_space_extent: usize,
    /// vm-sapce size (currently only used by jikesrvm)
    pub vm_space_size: usize,
    /// Number of bits to shift a space index into/out of a virtual address.
    /// In a 32-bit model, use a dummy value so that the compiler doesn't barf.
    pub space_shift_64: usize,
    /// Bitwise mask to isolate a space index in a virtual address.
    /// We can't express this constant in a 32-bit environment, hence the
    /// conditional definition.
    pub space_mask_64: usize,
    /// Size of each space in the 64-bit memory layout
    /// We can't express this constant in a 32-bit environment, hence the
    /// conditional definition.
    /// FIXME: When Compiling for 32 bits this expression makes no sense
    pub space_size_64: usize,
}

impl VMLayoutConstants {
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
        1 << self.log_max_chunks
    }
    /// Force contiguous virtual memory for all spaces
    pub fn force_use_contiguous_spaces(&self) -> bool {
        self.log_address_space > 35
    }
}

impl VMLayoutConstants {
    /// Normal 32-bit configuration
    pub const fn new_32bit() -> Self {
        unimplemented!()
    }
    /// Normal 64-bit configuration
    pub fn new_64bit() -> Self {
        Self {
            log_address_space: 47,
            heap_start: chunk_align_down(unsafe {
                Address::from_usize(0x0000_0200_0000_0000usize)
            }),
            heap_end: chunk_align_up(unsafe { Address::from_usize(0x0000_2200_0000_0000usize) }),
            vm_space_size: chunk_align_up(unsafe { Address::from_usize(0xdc0_0000) }).as_usize(),
            log_max_chunks: Self::LOG_ARCH_ADDRESS_SPACE - LOG_BYTES_IN_CHUNK,
            log_space_extent: 41,
            space_shift_64: 41,
            space_mask_64: ((1 << 4) - 1) << 41,
            space_size_64: 1 << 41,
        }
    }
    /// 64-bit configuration with compressed pointers
    pub fn new_64bit_with_pointer_compression(heap_size: usize) -> Self {
        assert!(
            heap_size <= (32usize << LOG_BYTES_IN_GBYTE),
            "Heap size is larger than 32 GB"
        );
        let start = 0x4000_0000;
        let end = match start + heap_size {
            end if end <= (4usize << 30) => 4usize << 30,
            end if end <= (32usize << 30) => 32usize << 30,
            _ => 0x4000_0000 + (32usize << 30),
        };
        Self {
            log_address_space: 35,
            heap_start: chunk_align_down(unsafe { Address::from_usize(start) }),
            heap_end: chunk_align_up(unsafe { Address::from_usize(end) }),
            vm_space_size: chunk_align_up(unsafe { Address::from_usize(0x800_0000) }).as_usize(),
            log_max_chunks: Self::LOG_ARCH_ADDRESS_SPACE - LOG_BYTES_IN_CHUNK,
            log_space_extent: 31,
            space_shift_64: 0,
            space_mask_64: 0,
            space_size_64: 0,
        }
    }

    /// Initialize the address space
    pub fn set_address_space(kind: AddressSpaceKind) {
        let mut guard = ADDRESS_SPACE_KIND.lock();
        assert!(guard.is_none(), "Address space can only be set once");
        *guard = Some(kind);
    }

    /// Get current address space
    pub fn get_address_space() -> AddressSpaceKind {
        ADDRESS_SPACE_KIND.lock().unwrap()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AddressSpaceKind {
    AddressSpace32Bit,
    AddressSpace64Bit,
    AddressSpace64BitWithPointerCompression { max_heap_size: usize },
}

impl AddressSpaceKind {
    pub const fn pointer_compression(&self) -> bool {
        match self {
            Self::AddressSpace64BitWithPointerCompression { .. } => true,
            _ => false,
        }
    }
}

static ADDRESS_SPACE_KIND: Mutex<Option<AddressSpaceKind>> = Mutex::new(None);

lazy_static! {
    pub static ref VM_LAYOUT_CONSTANTS: VMLayoutConstants = {
        let las = ADDRESS_SPACE_KIND
            .lock()
            .unwrap_or(if cfg!(target_pointer_width = "32") {
                AddressSpaceKind::AddressSpace32Bit
            } else {
                AddressSpaceKind::AddressSpace64Bit
            });
        match las {
            AddressSpaceKind::AddressSpace32Bit => unimplemented!(),
            AddressSpaceKind::AddressSpace64Bit => VMLayoutConstants::new_64bit(),
            AddressSpaceKind::AddressSpace64BitWithPointerCompression { max_heap_size } => {
                VMLayoutConstants::new_64bit_with_pointer_compression(max_heap_size)
            }
        }
    };
}
