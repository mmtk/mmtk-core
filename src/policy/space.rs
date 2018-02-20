use ::util::Address;
use ::util::ObjectReference;

use ::vm::{ActivePlan, VMActivePlan, Collection, VMCollection};
use ::util::heap::{VMRequest, PageResource};
use ::util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END, AVAILABLE_BYTES};

use ::plan::selected_plan::PLAN;

use std::sync::atomic::{AtomicUsize, Ordering};

use ::util::constants::LOG_BYTES_IN_MBYTE;

pub trait Space<'a, PR: PageResource<'a, Self>> {
    fn init(&mut self) {
        self.common_mut().pr.as_ref().unwrap().bind_space(
            unsafe{&*(self as *const Self)});
            // ^ Borrow-checker fighting so that we can have a cyclic reference
    }

    fn acquire(&self, thread_id: usize, pages: usize) -> Address {
        let allow_poll = unsafe { VMActivePlan::is_mutator(thread_id) }
            && PLAN::is_initialized();

        let pr = self.common().pr.as_ref().unwrap();
        let pages_reserved = pr.reserve_pages(pages);

        if allow_poll && VMActivePlan::global().poll(false) {
            pr.clear_request(pages_reserved);
            VMCollection::block_for_gc(thread_id);
            unsafe { Address::zero() }
        } else {
            let rtn = pr.get_new_pages(pages_reserved,
                                       pages, self.common().zeroed);
            if rtn.is_zero() {
                if !allow_poll {
                    panic!("Physical allocation failed when polling not allowed!");
                }

                let gc_performed = VMActivePlan::global().poll(true);
                debug_assert!(gc_performed, "GC not performed when forced.");
                pr.clear_request(pages_reserved);
                VMCollection::block_for_gc(thread_id);
                unsafe { Address::zero() }
            } else {
                rtn
            }
        }
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        object.value() >= self.common().start.as_usize()
            && object.value() < self.common().start.as_usize() + self.common().extent
    }

    fn common(&self) -> &CommonSpace<Self, PR>;

    fn common_mut(&mut self) -> &mut CommonSpace<Self, PR>;
}

pub struct CommonSpace<'a, S: Space<'a, PR>, PR: PageResource<'a, S>> {
    name: &'static str,
    name_length: usize,
    descriptor: usize,
    index: usize,
    vmrequest: VMRequest,

    immortal: bool,
    movable: bool,
    contiguous: bool,
    pub zeroed: bool,

    pub pr: Option<PR>,
    pub start: Address,
    pub extent: usize,
    head_discontiguous_region: Address,
}

static mut SPACE_COUNT: usize = 0;
static mut HEAP_CURSOR: Address = HEAP_START;
static mut HEAP_LIMIT: Address = HEAP_END;

const DEBUG: bool = false;

impl<'a, S: Space<'a, PR>, PR: PageResource<'a, S>> CommonSpace<'a, S, PR> {
    pub fn new(name: &'static str, movable: bool, immortal: bool, zeroed: bool,
               vmrequest: VMRequest) -> Self {
        let mut rtn = CommonSpace {
            name,
            name_length: name.len(),
            descriptor: 0,
            index: unsafe { let tmp = SPACE_COUNT; SPACE_COUNT += 1; tmp },
            vmrequest,
            immortal,
            movable,
            contiguous: true,
            zeroed,
            pr: None,
            start: unsafe{Address::zero()},
            extent: 0,
            head_discontiguous_region: unsafe{Address::zero()},
        };

        if vmrequest.is_discontiguous() {
            rtn.contiguous = false;
            // FIXME
            // rtn.descriptor = SpaceDescriptor.createDescriptor()
            // VM.memory.setHeapRange(index, HEAP_START, HEAP_END);
            return rtn;
        }

        let (extent, top) = match vmrequest {
            VMRequest::RequestFraction{frac, top: _top}                   => (get_frac_available(frac), _top),
            VMRequest::RequestExtent{extent: _extent, top: _top}          => (_extent, _top),
            VMRequest::RequestFixed{start: _, extent: _extent, top: _top} => (_extent, _top),
            _                                                             => unreachable!(),
        };

        if extent != chunk_align!(extent, false) {
            panic!("{} requested non-aligned extent: {} bytes", name, extent);
        }

        let start: Address;
        if let VMRequest::RequestFixed{start: _start, extent: _, top: _} = vmrequest {
            start = _start;
            if start.as_usize() != chunk_align!(start.as_usize(), false) {
                panic!("{} starting on non-aligned boundary: {} bytes", name, start.as_usize());
            }
        } else if top {
            // FIXME
            //if (HeapLayout.vmMap.isFinalized()) VM.assertions.fail("heap is narrowed after regionMap is finalized: " + name);
            unsafe {
                HEAP_LIMIT -= extent;
                start = HEAP_LIMIT;
            }
        } else {
            unsafe {
                start = HEAP_CURSOR;
                HEAP_CURSOR += extent;
            }
        }

        if HEAP_CURSOR > HEAP_LIMIT {
            unsafe {
                panic!("Out of virtual address space allocating \"{}\" at {} ({} > {})", name,
                       HEAP_CURSOR - extent, HEAP_CURSOR, HEAP_LIMIT);
            }
        }

        rtn.contiguous = true;
        rtn.start = start;
        rtn.extent = extent;
        // FIXME
        // rtn.descriptor = SpaceDescriptor.createDescriptor()
        // VM.memory.setHeapRange(index, start, start.plus(extent));
        // HeapLayout.vmMap.insert(start, extent, descriptor, this);

        if DEBUG {
            debug!("{} {} {} {}", name, start, start + extent, extent);
        }

        rtn
    }
}

fn get_frac_available(frac: f32) -> usize {
    let bytes = frac * AVAILABLE_BYTES;
    let mb = bytes >> LOG_BYTES_IN_MBYTE;
    let rtn = mb << LOG_BYTES_IN_MBYTE;
    chunk_align!(rtn, false)
}