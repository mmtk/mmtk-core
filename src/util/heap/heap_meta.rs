use std::cell::RefCell;
use std::rc::Rc;

use crate::util::heap::layout::vm_layout::vm_layout;
use crate::util::Address;

/// This struct is used to determine the placement of each space during the creation of a Plan.
///
/// TODO: This type needs a better name.
pub struct HeapMeta {
    pub heap_cursor: Address,
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

/// This struct represents the placement decision of a space.
pub struct SpaceMeta {
    pub start: Address,
    pub extent: usize,
    pub is_contiguous: bool,
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
            heap_cursor: vm_layout().heap_start,
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

    pub fn reserve(&mut self, extent: usize, top: bool) -> Address {
        let ret = if top {
            self.heap_limit -= extent;
            self.heap_limit
        } else {
            let start = self.heap_cursor;
            self.heap_cursor += extent;
            start
        };

        assert!(
            self.heap_cursor <= self.heap_limit,
            "Out of virtual address space at {} ({} > {})",
            self.heap_cursor - extent,
            self.heap_cursor,
            self.heap_limit
        );

        ret
    }

    pub fn get_discontig_start(&self) -> Address {
        self.heap_cursor
    }

    pub fn get_discontig_end(&self) -> Address {
        self.heap_limit - 1
    }
}

// make clippy happy
impl Default for HeapMeta {
    fn default() -> Self {
        Self::new()
    }
}
