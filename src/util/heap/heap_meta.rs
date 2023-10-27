use std::cell::RefCell;
use std::rc::Rc;

use crate::util::constants::LOG_BYTES_IN_MBYTE;
use crate::util::conversions::raw_align_up;
use crate::util::heap::layout::vm_layout::vm_layout;
use crate::util::heap::vm_layout::BYTES_IN_CHUNK;
use crate::util::Address;

/// This struct is used to determine the placement of each space during the creation of a Plan.
///
/// TODO: This type needs a better name.
pub struct HeapMeta {
    pub heap_start: Address,
    pub heap_limit: Address,
    entries: Vec<SpaceEntry>,
}

struct SpaceEntry {
    spec: SpaceSpec,
    promise_meta: PromiseSpaceMeta,
}

/// This enum specifies the requirement of space placement.
///
/// Note that the result of space placement (represented by `SpaceMeta`) may give the space a
/// larger address range than requested.  For example, on systems with a generous address space,
/// the space placement strategy may give each space a contiguous 2TiB address space even if it
/// requests a small extent.
pub enum SpaceSpec {
    /// There is no size or place requirement for the space.  The space may be given a very large
    /// contiguous or discontiguous space range of address, depending on the strategy.
    DontCare,
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

impl SpaceSpec {
    fn dont_care(&self) -> bool {
        matches!(self, SpaceSpec::DontCare)
    }

    fn top(&self) -> bool {
        match *self {
            SpaceSpec::DontCare => false,
            SpaceSpec::Extent { top, .. } => top,
            SpaceSpec::Fraction { top, .. } => top,
        }
    }
}

/// This struct represents the placement decision of a space.
pub struct SpaceMeta {
    pub space_id: usize,
    pub start: Address,
    pub extent: usize,
    pub contiguous: bool,
}

/// A space meta that will be provided in the future.
#[derive(Clone)]
pub struct FutureSpaceMeta {
    inner: Rc<RefCell<Option<SpaceMeta>>>,
}

impl FutureSpaceMeta {
    /// Unwrap `self` and get a `SpaceMeta` instance.  Can only be called after calling
    /// `HeapMeta::place_spaces()`.
    pub fn unwrap(self) -> SpaceMeta {
        let mut opt = self.inner.borrow_mut();
        opt.take()
            .expect("Attempt to get SpaceMeta before calling HeapMeta::place_spaces()")
    }
}

/// The struct for HeapMeta to provide a SpaceMeta instance for its user.
struct PromiseSpaceMeta {
    inner: Rc<RefCell<Option<SpaceMeta>>>,
}

impl PromiseSpaceMeta {
    fn provide(&mut self, space_meta: SpaceMeta) {
        let mut opt = self.inner.borrow_mut();
        assert!(opt.is_none());
        *opt = Some(space_meta);
    }
}

impl HeapMeta {
    pub fn new() -> Self {
        HeapMeta {
            heap_start: vm_layout().heap_start,
            heap_limit: vm_layout().heap_end,
            entries: Vec::default(),
        }
    }

    pub fn specify_space(&mut self, spec: SpaceSpec) -> FutureSpaceMeta {
        let shared_meta = Rc::new(RefCell::new(None));
        let future_meta = FutureSpaceMeta {
            inner: shared_meta.clone(),
        };
        let promise_meta = PromiseSpaceMeta { inner: shared_meta };
        self.entries.push(SpaceEntry { spec, promise_meta });
        future_meta
    }

    pub fn place_spaces(&mut self) {
        let force_use_contiguous_spaces = vm_layout().force_use_contiguous_spaces;

        let mut reserver = AddressRangeReserver::new(self.heap_start, self.heap_limit);

        if force_use_contiguous_spaces {
            let extent = vm_layout().max_space_extent();

            for (i, entry) in self.entries.iter_mut().enumerate() {
                let top = entry.spec.top();
                let start = reserver.reserve(extent, top);

                let meta = SpaceMeta {
                    space_id: i,
                    start,
                    extent,
                    contiguous: true,
                };

                entry.promise_meta.provide(meta);
            }
        } else {
            for (i, entry) in self.entries.iter_mut().enumerate() {
                let (start, extent) = match entry.spec {
                    SpaceSpec::DontCare => continue,
                    SpaceSpec::Extent { extent, top } => {
                        let start = reserver.reserve(extent, top);
                        (start, extent)
                    }
                    SpaceSpec::Fraction { frac, top } => {
                        // Taken from `crate::policy::space::get_frac_available`, but we currently
                        // don't have any plans that actually uses it.
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

                let meta = SpaceMeta {
                    space_id: i,
                    start,
                    extent,
                    contiguous: true,
                };

                entry.promise_meta.provide(meta);
            }

            let (discontig_start, discontig_end) = reserver.remaining_range();
            let discontig_extent = discontig_end - discontig_start;
            for (i, entry) in self.entries.iter_mut().enumerate() {
                if !entry.spec.dont_care() {
                    continue;
                }

                let meta = SpaceMeta {
                    space_id: i,
                    start: discontig_start,
                    extent: discontig_extent,
                    contiguous: false,
                };

                entry.promise_meta.provide(meta);
            }
        }
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

    pub fn remaining_range(&self) -> (Address, Address) {
        (self.lower_bound, self.upper_bound)
    }
}
