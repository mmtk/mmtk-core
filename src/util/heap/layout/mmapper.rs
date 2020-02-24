use ::util::{Address, ObjectReference};
use ::util::heap::layout::vm_layout_constants::*;

pub const MMAP_CHUNK_BYTES: usize = 1 << LOG_MMAP_CHUNK_BYTES;   // the granularity VMResource operates at
pub const MMAP_CHUNK_MASK: usize = MMAP_CHUNK_BYTES - 1;

pub trait Mmapper {
    /****************************************************************************
     * Generic mmap and protection functionality
     */

    /**
     * Given an address array describing the regions of virtual memory to be used
     * by MMTk, demand zero map all of them if they are not already mapped.
     *
     * @param spaceMap An address array containing a pairs of start and end
     * addresses for each of the regions to be mappe3d
     */
    fn eagerly_mmap_all_spaces(&self, space_map: &[Address]);

    /**
     * Mark a number of pages as mapped, without making any
     * request to the operating system.  Used to mark pages
     * that the VM has already mapped.
     * @param start Address of the first page to be mapped
     * @param bytes Number of bytes to ensure mapped
     */
    fn mark_as_mapped(&self, start: Address, bytes: usize);

    /**
     * Ensure that a range of pages is mmapped (or equivalent).  If the
     * pages are not yet mapped, demand-zero map them. Note that mapping
     * occurs at chunk granularity, not page granularity.<p>
     *
     * NOTE: There is a monotonicity assumption so that only updates require lock
     * acquisition.
     * TODO: Fix the above to support unmapping.
     *
     * @param start The start of the range to be mapped.
     * @param pages The size of the range to be mapped, in pages
     */
    fn ensure_mapped(&self, start: Address, pages: usize);

    /**
     * Is the page pointed to by this address mapped ?
     * @param addr Address in question
     * @return {@code true} if the page at the given address is mapped.
     */
    fn address_is_mapped(&self, addr: Address) -> bool;

    /**
     * Mark a number of pages as inaccessible.
     * @param start Address of the first page to be protected
     * @param pages Number of pages to be protected
     */
    fn protect(&self, start: Address, pages: usize);
}