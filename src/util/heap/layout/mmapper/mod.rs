use crate::util::{
    memory::{MmapAnnotation, MmapStrategy},
    Address,
};
use std::io::Result;

#[allow(unused)] // Used in doc comment.
use crate::util::constants::LOG_BYTES_IN_PAGE;

pub mod csm;

/// An `Mmapper` manages the mmap state of memory used by the heap and side metadata of MMTk.
///
/// For the efficiency of implementation, an `Mmapper` operates at the granularity of
/// [`Mmapper::granularity()`].  Methods that take memory ranges as arguments will round the range
/// to the overlapping chunks.
pub trait Mmapper: Sync {
    /// The logarithm of granularity of this `Mmapper`, in bytes.  Must be at least
    /// [`LOG_BYTES_IN_PAGE`].
    ///
    /// See trait-level doc for [`Mmapper`] for details.
    fn log_granularity(&self) -> u8;

    /// The granularity of `Mmapper`.  Don't override this method.  Override
    /// [`Mmapper::log_granularity`] instead.
    ///
    /// See trait-level doc for [`Mmapper`] for details.
    fn granularity(&self) -> usize {
        1 << self.log_granularity()
    }

    /// Given an address array describing the regions of virtual memory to be used
    /// by MMTk, demand zero map all of them if they are not already mapped.
    ///
    /// Arguments:
    /// * `spaceMap`: An address array containing a pairs of start and end
    ///   addresses for each of the regions to be mapped
    fn eagerly_mmap_all_spaces(&self, _space_map: &[Address]);

    /// Mark a number of pages as mapped, without making any
    /// request to the operating system.  Used to mark pages
    /// that the VM has already mapped.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be mapped
    /// * `bytes`: Number of bytes to ensure mapped
    fn mark_as_mapped(&self, start: Address, bytes: usize);

    /// Quarantine/reserve address range. We mmap from the OS with no reserve and with PROT_NONE,
    /// which should be little overhead. This ensures that we can reserve certain address range that
    /// we can use if needed. Quarantined memory needs to be mapped before it can be used.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be quarantined
    /// * `pages`: Number of pages to quarantine from the start
    /// * `strategy`: The mmap strategy.  The `prot` field is ignored because we always use
    ///   `PROT_NONE`.
    /// * `anno`: Human-readable annotation to apply to newly mapped memory ranges.
    fn quarantine_address_range(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()>;

    /// Ensure that a range of pages is mmapped (or equivalent).  If the
    /// pages are not yet mapped, demand-zero map them. Note that mapping
    /// occurs at chunk granularity, not page granularity.
    ///
    /// Arguments:
    /// * `start`: The start of the range to be mapped.
    /// * `pages`: The size of the range to be mapped, in pages
    /// * `strategy`: The mmap strategy.
    /// * `anno`: Human-readable annotation to apply to newly mapped memory ranges.
    // NOTE: There is a monotonicity assumption so that only updates require lock
    // acquisition.
    // TODO: Fix the above to support unmapping.
    fn ensure_mapped(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()>;

    /// Is the page pointed to by this address mapped? Returns true if
    /// the page at the given address is mapped.
    ///
    /// Arguments:
    /// * `addr`: Address in question
    fn is_mapped_address(&self, addr: Address) -> bool;

    /// Mark a number of pages as inaccessible.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be protected
    /// * `pages`: Number of pages to be protected
    fn protect(&self, start: Address, pages: usize);
}
