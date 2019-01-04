use ::util::Address;
use ::util::constants::*;
use super::heap_parameters::*;

///////// FIXME ////////////
use super::super::vmrequest::{HEAP_LAYOUT_32BIT, HEAP_LAYOUT_64BIT};

/**
 * Enable messages in the BootImageWriter log file
 */
const VERBOSE_BUILD: bool = true;

/** log_2 of the addressable virtual space */
pub const LOG_ADDRESS_SPACE: usize = if_then_else_usize!(HEAP_LAYOUT_32BIT, 32,
    LOG_SPACE_SIZE_64 + LOG_MAX_SPACES);
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

/** Coarsest unit of address space allocation, in pages */
pub const PAGES_IN_CHUNK: usize = 1 << (LOG_BYTES_IN_CHUNK as usize - LOG_BYTES_IN_PAGE as usize);

/** log_2 of the maximum number of chunks we need to track.  Only used in 32-bit layout.*/
pub const LOG_MAX_CHUNKS: usize = LOG_ADDRESS_SPACE - LOG_BYTES_IN_CHUNK;

/** Maximum number of chunks we need to track.  Only used in 32-bit layout. */
pub const MAX_CHUNKS: usize = 1 << LOG_MAX_CHUNKS;

/**
 * An upper bound on the extent of any space in the
 * current memory layout
 */
pub const LOG_SPACE_EXTENT: usize = if_then_else_usize!(HEAP_LAYOUT_64BIT, LOG_SPACE_SIZE_64, 31);

/**
 * An upper bound on the extent of any space in the
 * current memory layout
 */
pub const MAX_SPACE_EXTENT: usize = 1 << LOG_SPACE_EXTENT;

// FIXME: HEAP_START, HEAP_END are VM-dependent
/** Lowest virtual address used by the virtual machine */
pub const HEAP_START: Address = unsafe{Address::from_usize(chunk_align!(0x60000000, true))};

/** Highest virtual address used by the virtual machine */
pub const HEAP_END: Address = unsafe{Address::from_usize(chunk_align!(0xb0000000, false))};

/**
 * Lowest virtual address available for MMTk to manage.  The address space between
 * HEAP_START and AVAILABLE_START comprises memory directly managed by the VM,
 * and not available to MMTk.
 */
pub const AVAILABLE_START: Address = unsafe{Address::from_usize(
    chunk_align!(0x67000000 + (0x64000000 - 0x60000000)/5, false))};

/**
 * Highest virtual address available for MMTk to manage.  The address space between
 * HEAP_END and AVAILABLE_END comprises memory directly managed by the VM,
 * and not available to MMTk.
*/
pub const AVAILABLE_END: Address = unsafe{Address::from_usize(chunk_align!(0xb0000000, true))};

/** Size of the address space available to the MMTk heap. */
pub const AVAILABLE_BYTES: usize = AVAILABLE_END.as_usize() - AVAILABLE_START.as_usize();

/** Granularity at which we map and unmap virtual address space in the heap */
pub const LOG_MMAP_CHUNK_BYTES: usize = 20;

/** log_2 of the number of pages in a 64-bit space */
pub const LOG_PAGES_IN_SPACE64: usize = LOG_SPACE_SIZE_64 as usize - LOG_BYTES_IN_PAGE as usize;

/** The number of pages in a 64-bit space */
pub const PAGES_IN_SPACE64: usize = 1 << LOG_PAGES_IN_SPACE64;

/*
 *  The 64-bit VM layout divides address space into LOG_MAX_SPACES (k) fixed size
 *  regions of size 2^n, aligned at 2^n byte boundaries.  A virtual address can be
 *  subdivided into fields as follows
 *
 *    64                              0
 *    00...0SSSSSaaaaaaaaaaa...aaaaaaaa
 *
 * The field 'S' identifies the space to which the address points.
 */

/**
 * Number of bits to shift a space index into/out of a virtual address.
 */
/* In a 32-bit model, use a dummy value so that the compiler doesn't barf. */
pub const SPACE_SHIFT_64: usize = if_then_else_usize!(HEAP_LAYOUT_64BIT, LOG_SPACE_SIZE_64, 0);

/**
 * Bitwise mask to isolate a space index in a virtual address.
 *
 * We can't express this constant in a 32-bit environment, hence the
 * conditional definition.
 */
pub const SPACE_MASK_64: usize = if_then_else_zero_usize!(HEAP_LAYOUT_64BIT,
    ((1 << LOG_MAX_SPACES) - 1) << SPACE_SHIFT_64);

/*
 * Size of each space in the 64-bit memory layout
 *
 * We can't express this constant in a 32-bit environment, hence the
 * conditional definition.
 */
// FIXME: When Compiling for 32 bits this expression makes no sense
// #[allow(const_err)]
// pub const SPACE_SIZE_64: usize = if_then_else_usize!(HEAP_LAYOUT_64BIT,
//    1 << LOG_SPACE_SIZE_64, MAX_SPACE_EXTENT);
#[cfg(target_pointer_width = "64")]
pub const SPACE_SIZE_64: usize = 1 << LOG_SPACE_SIZE_64;
#[cfg(target_pointer_width = "32")]
pub const SPACE_SIZE_64: usize = MAX_SPACE_EXTENT;