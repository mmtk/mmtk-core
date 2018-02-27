use ::util::Address;
use ::util::ObjectReference;
use ::util::conversions::*;

use ::vm::{ActivePlan, VMActivePlan, Collection, VMCollection};
use ::util::heap::{VMRequest, PageResource};
use ::util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END, AVAILABLE_BYTES, LOG_BYTES_IN_CHUNK};
use ::util::heap::layout::vm_layout_constants::{AVAILABLE_START, AVAILABLE_END};

use ::plan::Plan;
use ::plan::selected_plan::PLAN;

use std::sync::atomic::{AtomicUsize, Ordering};

use ::util::constants::LOG_BYTES_IN_MBYTE;

use std::marker::PhantomData;

pub trait Space<PR: PageResource<Self>>: Sized + 'static {
    fn init(&mut self);

    fn acquire(&self, thread_id: usize, pages: usize) -> Address {
        let allow_poll = unsafe { VMActivePlan::is_mutator(thread_id) }
            && PLAN.is_initialized();

        let pr = self.common().pr.as_ref().unwrap();
        let pages_reserved = pr.reserve_pages(pages);

        // FIXME: Possibly unnecessary borrow-checker fighting
        let me = unsafe { &*(self as *const Self) };

        if allow_poll && VMActivePlan::global().poll(false, me) {
            pr.clear_request(pages_reserved);
            VMCollection::block_for_gc(thread_id);
            unsafe { Address::zero() }
        } else {
            let rtn = pr.get_new_pages(pages_reserved, pages, self.common().zeroed, thread_id);
            if rtn.is_zero() {
                if !allow_poll {
                    panic!("Physical allocation failed when polling not allowed!");
                }

                let gc_performed = VMActivePlan::global().poll(true, me);
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

    fn grow_discontiguous_space(&self, chunks: usize) -> Address {
        // FIXME
        let new_head: Address = unimplemented!(); /*HeapLayout.vmMap. allocate_contiguous_chunks(self.common().descriptor,
                                                                        self, chunks,
                                                                        self.common().head_discontiguous_region);*/
        if new_head.is_zero() {
            return unsafe{Address::zero()};
        }

        self.common_mut().head_discontiguous_region = new_head;
        new_head
    }

    /**
     * This hook is called by page resources each time a space grows.  The space may
     * tap into the hook to monitor heap growth.  The call is made from within the
     * page resources' critical region, immediately before yielding the lock.
     *
     * @param start The start of the newly allocated space
     * @param bytes The size of the newly allocated space
     * @param new_chunk {@code true} if the new space encroached upon or started a new chunk or chunks.
     */
    fn grow_space(&self, start: Address, bytes: usize, new_chunk: bool) {}

    fn common(&self) -> &CommonSpace<Self, PR>;

    fn common_mut(&self) -> &mut CommonSpace<Self, PR>;
}

pub struct CommonSpace<S: Space<PR>, PR: PageResource<S>> {
    pub name: &'static str,
    name_length: usize,
    pub descriptor: usize,
    index: usize,
    pub vmrequest: VMRequest,

    immortal: bool,
    movable: bool,
    pub contiguous: bool,
    pub zeroed: bool,

    pub pr: Option<PR>,
    pub start: Address,
    pub extent: usize,
    pub head_discontiguous_region: Address,

    _placeholder: PhantomData<S>,
}

static mut SPACE_COUNT: usize = 0;
static mut HEAP_CURSOR: Address = HEAP_START;
static mut HEAP_LIMIT: Address = HEAP_END;

const DEBUG: bool = false;

impl<S: Space<PR>, PR: PageResource<S>> CommonSpace<S, PR> {
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
            _placeholder: PhantomData,
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

        if extent != raw_chunk_align(extent, false) {
            panic!("{} requested non-aligned extent: {} bytes", name, extent);
        }

        let start: Address;
        if let VMRequest::RequestFixed{start: _start, extent: _, top: _} = vmrequest {
            start = _start;
            if start.as_usize() != chunk_align(start, false).as_usize() {
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

        unsafe {
            if HEAP_CURSOR > HEAP_LIMIT {
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
    trace!("AVAILABLE_START={}", AVAILABLE_START);
    trace!("AVAILABLE_END={}", AVAILABLE_END);
    let bytes = (frac * AVAILABLE_BYTES as f32) as usize;
    trace!("bytes={}*{}={}", frac, AVAILABLE_BYTES, bytes);
    let mb = bytes >> LOG_BYTES_IN_MBYTE;
    let rtn = mb << LOG_BYTES_IN_MBYTE;
    trace!("rtn={}", rtn);
    let aligned_rtn = raw_chunk_align(rtn, false);
    trace!("aligned_rtn={}", aligned_rtn);
    aligned_rtn
}

pub fn required_chunks(pages: usize) -> usize {
    let extent = raw_chunk_align(pages_to_bytes(pages), false);
    extent >> LOG_BYTES_IN_CHUNK
}