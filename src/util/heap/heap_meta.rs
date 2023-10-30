//! This module determines the address ranges of spaces of a plan according to the specifications
//! given by the plan.
//!
//! [`HeapMeta`] is the helper type for space placement, and is a prerequisite of creating plans.
//! It is used as following.
//!
//! 1.  A plan declares all the spaces it wants to create using the `specify_space` method.  For
//!     each space, it passes a [`VMRequest`] which specifies the requirements for each space,
//!     including whether the space is contiguous, whether it has a fixed extent, and whether it
//!     should be place at the low end or high end of the heap range, etc.  The `specify_space`
//!     method returns a [`PendingVMResponse`] for each space which can be used later.
//! 2.  After all spaces are specified, the plan calls the `place_spaces` method.  It determines
//!     the locations (starts and extends) and contiguousness of all spaces according to the policy
//!     specified by [`crate::util::heap::layout::vm_layout::vm_layout`].
//! 3.  Then the plan calls `unwrap()` on each [`PendingVMResponse`] to get a [`VMResponse`] which
//!     holds the the placement decision for each space (start, extent, contiguousness, etc.).
//!     Using such information, the space can create each concrete spaces.
//!
//! In summary, the plan specifies all spaces before [`HeapMeta`] makes placement decision, and all
//! spaces know their locations the moment they are created.
//!
//! By doing so, we can avoid creating spaces first and then computing their start addresses and
//! mutate those spaces.  JikesRVM's MMTk used to do that, but such practice is unfriendly to Rust
//! which has strict ownership and mutability rules.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use crate::util::constants::LOG_BYTES_IN_MBYTE;
use crate::util::conversions::raw_align_up;
use crate::util::heap::layout::vm_layout::vm_layout;
use crate::util::heap::vm_layout::BYTES_IN_CHUNK;
use crate::util::Address;

/// This struct is used to determine the placement of each space during the creation of a Plan.
/// Read the module-level documentation for how to use.
///
/// TODO: This type needs a better name.
pub struct HeapMeta {
    /// The start of the heap range (inclusive).
    heap_start: Address,
    /// The end of the heap range (exclusive).
    heap_limit: Address,
    /// The address range for discontiguous spaces (if exists).
    discontiguous_range: Option<Range<Address>>,
    /// Request-response pairs for each space.
    entries: Vec<SpaceEntry>,
}

/// A request-response pair.
struct SpaceEntry {
    req: VMRequest,
    resp: PendingVMResponseWriter,
}

/// A virtual memory (VM) request specifies the requirement for placing a space in the virtual
/// address space.  It will be processed by [`HeapMeta`].
///
/// Note that the result of space placement (represented by [`VMResponse`]) may give the space a
/// larger address range than requested.  For example, on systems with a generous address space,
/// the space placement strategy may give each space a contiguous 2TiB address space even if it
/// requests a small extent.
#[derive(Debug)]
pub enum VMRequest {
    /// There is no size, location, or contiguousness requirement for the space.  In a confined
    /// address space, the space may be given a discontiguous address range shared with other
    /// spaces; in a generous address space, the space may be given a very large contiguous address
    /// range solely owned by this space.
    Unrestricted,
    /// Require a contiguous range of address of a fixed size.
    Extent {
        /// The size of the space, in bytes.  Must be a multiple of chunks.
        extent: usize,
        /// `true` if the space should be placed at the high end of the heap range; `false` if it
        /// should be placed at the low end of the heap range.
        top: bool,
    },
    /// Require a contiguous range of address, and its size should be a fraction of the total heap
    /// size.
    Fraction {
        /// The size of the space as a fraction of the heap size.  The size will be rounded to a
        /// multiple of chunks.
        frac: f32,
        /// `true` if the space should be placed at the high end of the heap range; `false` if it
        /// should be placed at the low end of the heap range.
        top: bool,
    },
}

impl VMRequest {
    /// Return `true` if the current `VMRequest` is unrestricted.
    fn unrestricted(&self) -> bool {
        matches!(self, VMRequest::Unrestricted)
    }

    /// Return `true` if the space should be placed at the high end of the address space.
    fn top(&self) -> bool {
        match *self {
            VMRequest::Unrestricted => false,
            VMRequest::Extent { top, .. } => top,
            VMRequest::Fraction { top, .. } => top,
        }
    }
}

/// This struct represents the placement decision of a space.
#[derive(Debug)]
pub struct VMResponse {
    /// An assigned ID of the space.  Guaranteed to be unique.
    pub space_id: usize,
    /// The start of the address range of the space.  For discontiguous spaces, this range will be
    /// shared with other discontiguous spaces.
    pub start: Address,
    /// The extent of the address range of the space.
    pub extent: usize,
    /// `true` if the space is contiguous.
    pub contiguous: bool,
}

impl VMResponse {
    /// Create a dummy `VMResponse` for `VMSpace` because the address range of `VMSpace` is not
    /// determined by `HeapMeta`.
    pub(crate) fn vm_space_dummy() -> Self {
        Self {
            space_id: usize::MAX,
            start: Address::ZERO,
            extent: 0,
            contiguous: false,
        }
    }
}

/// A `VMResponse` that will be provided in the future.
#[derive(Clone)]
pub struct PendingVMResponse {
    inner: Rc<RefCell<Option<VMResponse>>>,
}

impl PendingVMResponse {
    /// Unwrap `self` and get a `VMResponse` instance.  Can only be called after calling
    /// `HeapMeta::place_spaces()`.
    pub fn unwrap(self) -> VMResponse {
        let mut opt = self.inner.borrow_mut();
        opt.take()
            .expect("Attempt to get VMResponse before calling HeapMeta::place_spaces()")
    }
}

/// The struct for `HeapMeta` to provide a `VMResponse` instance for its user.
struct PendingVMResponseWriter {
    inner: Rc<RefCell<Option<VMResponse>>>,
}

impl PendingVMResponseWriter {
    fn provide(&mut self, resp: VMResponse) {
        let mut opt = self.inner.borrow_mut();
        assert!(opt.is_none());
        *opt = Some(resp);
    }
}

impl HeapMeta {
    /// Create a `HeapMeta` instance.  The heap range will be determined by
    /// [`crate::util::heap::layout::vm_layout::vm_layout`].
    pub fn new() -> Self {
        HeapMeta {
            heap_start: vm_layout().heap_start,
            heap_limit: vm_layout().heap_end,
            entries: Vec::default(),
            discontiguous_range: None,
        }
    }

    /// Declare a space and specify the detailed requirements.
    pub fn specify_space(&mut self, req: VMRequest) -> PendingVMResponse {
        let shared_resp = Rc::new(RefCell::new(None));
        let pending_resp = PendingVMResponse {
            inner: shared_resp.clone(),
        };
        let resp = PendingVMResponseWriter { inner: shared_resp };
        self.entries.push(SpaceEntry { req, resp });
        pending_resp
    }

    /// Determine the locations of all specified spaces.
    pub fn place_spaces(&mut self) {
        let force_use_contiguous_spaces = vm_layout().force_use_contiguous_spaces;

        let mut reserver = AddressRangeReserver::new(self.heap_start, self.heap_limit);

        if force_use_contiguous_spaces {
            debug!(
                "Placing spaces in a generous address space: [{}, {})",
                self.heap_start, self.heap_limit
            );
            let extent = vm_layout().max_space_extent();

            for (i, entry) in self.entries.iter_mut().enumerate() {
                let top = entry.req.top();
                let start = reserver.reserve(extent, top);

                let resp = VMResponse {
                    space_id: i,
                    start,
                    extent,
                    contiguous: true,
                };

                debug!("  VMResponse: {:?}", resp);
                entry.resp.provide(resp);
            }
        } else {
            debug!(
                "Placing spaces in a confined address space: [{}, {})",
                self.heap_start, self.heap_limit
            );
            for (i, entry) in self.entries.iter_mut().enumerate() {
                let (start, extent) = match entry.req {
                    VMRequest::Unrestricted => continue,
                    VMRequest::Extent { extent, top } => {
                        let start = reserver.reserve(extent, top);
                        (start, extent)
                    }
                    VMRequest::Fraction { frac, top } => {
                        // Taken from `crate::policy::space::get_frac_available`, but we currently
                        // don't have any plans that actually use it.
                        let extent = {
                            trace!("AVAILABLE_START={}", self.heap_start);
                            trace!("AVAILABLE_END={}", self.heap_limit);
                            let available_bytes = self.heap_limit - self.heap_start;
                            let bytes = (frac * available_bytes as f32) as usize;
                            trace!("bytes={}*{}={}", frac, vm_layout().available_bytes(), bytes);
                            let mb = bytes >> LOG_BYTES_IN_MBYTE;
                            let rtn = mb << LOG_BYTES_IN_MBYTE;
                            trace!("rtn={}", rtn);
                            let aligned_rtn = raw_align_up(rtn, BYTES_IN_CHUNK);
                            trace!("aligned_rtn={}", aligned_rtn);
                            aligned_rtn
                        };
                        let start = reserver.reserve(extent, top);
                        (start, extent)
                    }
                };

                let resp = VMResponse {
                    space_id: i,
                    start,
                    extent,
                    contiguous: true,
                };

                debug!("  VMResponse: {:?}", resp);
                entry.resp.provide(resp);
            }

            let discontig_range = reserver.remaining_range();
            self.discontiguous_range = Some(discontig_range.clone());
            let Range {
                start: discontig_start,
                end: discontig_end,
            } = discontig_range;

            debug!(
                "Discontiguous range is [{}, {})",
                discontig_start, discontig_end
            );

            let discontig_extent = discontig_end - discontig_start;
            for (i, entry) in self.entries.iter_mut().enumerate() {
                if !entry.req.unrestricted() {
                    continue;
                }

                let resp = VMResponse {
                    space_id: i,
                    start: discontig_start,
                    extent: discontig_extent,
                    contiguous: false,
                };

                debug!("  VMResponse: {:?}", resp);
                entry.resp.provide(resp);
            }
        }

        debug!("Space placement finished.");
    }

    /// Get the shared address range for discontigous spaces.
    pub fn get_discontiguous_range(&self) -> Option<Range<Address>> {
        self.discontiguous_range.clone()
    }
}

// make clippy happy
impl Default for HeapMeta {
    fn default() -> Self {
        Self::new()
    }
}

/// A helper struct for reserving spaces from both ends of an address region.
struct AddressRangeReserver {
    pub lower_bound: Address,
    pub upper_bound: Address,
}

impl AddressRangeReserver {
    pub fn new(lower_bound: Address, upper_bound: Address) -> Self {
        assert!(lower_bound.is_aligned_to(BYTES_IN_CHUNK));
        assert!(upper_bound.is_aligned_to(BYTES_IN_CHUNK));

        Self {
            lower_bound,
            upper_bound,
        }
    }

    pub fn reserve(&mut self, extent: usize, top: bool) -> Address {
        let ret = if top {
            self.upper_bound -= extent;
            self.upper_bound
        } else {
            let start = self.lower_bound;
            self.lower_bound += extent;
            start
        };

        assert!(
            self.lower_bound <= self.upper_bound,
            "Out of virtual address space at {} ({} > {})",
            self.lower_bound - extent,
            self.lower_bound,
            self.upper_bound
        );

        ret
    }

    pub fn remaining_range(&self) -> Range<Address> {
        self.lower_bound..self.upper_bound
    }
}
