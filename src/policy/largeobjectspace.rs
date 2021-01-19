use std::cell::UnsafeCell;

use crate::plan::PlanConstraints;
use crate::plan::TransitiveClosure;
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::constants::{BYTES_IN_PAGE, LOG_BYTES_IN_WORD};
use crate::util::gc_byte;
use crate::util::header_byte::HeaderByte;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::heap::{FreeListPageResource, PageResource, VMRequest};
use crate::util::treadmill::TreadMill;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;

#[allow(unused)]
const PAGE_MASK: usize = !(BYTES_IN_PAGE - 1);
const MARK_BIT: u8 = 0b01;
const NURSERY_BIT: u8 = 0b10;
const LOS_BIT_MASK: u8 = 0b11;

const USE_PRECEEDING_GC_HEADER: bool = true;
const PRECEEDING_GC_HEADER_WORDS: usize = 1;
const PRECEEDING_GC_HEADER_BYTES: usize = PRECEEDING_GC_HEADER_WORDS << LOG_BYTES_IN_WORD;

pub struct LargeObjectSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
    mark_state: u8,
    in_nursery_gc: bool,
    treadmill: TreadMill,
    header_byte: HeaderByte,
}

unsafe impl<VM: VMBinding> Sync for LargeObjectSpace<VM> {}

impl<VM: VMBinding> SFT for LargeObjectSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        self.test_mark_bit(object, self.mark_state)
    }
    fn is_movable(&self) -> bool {
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_header(&self, object: ObjectReference, alloc: bool) {
        let old_value = gc_byte::read_gc_byte::<VM>(object);
        let mut new_value = (old_value & (!LOS_BIT_MASK)) | self.mark_state;
        if alloc {
            new_value |= NURSERY_BIT;
        }
        gc_byte::write_gc_byte::<VM>(object, new_value);
        let cell = VM::VMObjectModel::object_start_ref(object)
            - if USE_PRECEEDING_GC_HEADER {
                PRECEEDING_GC_HEADER_BYTES
            } else {
                0
            };
        self.treadmill.add_to_treadmill(cell, alloc);
        if self.header_byte.needs_unlogged_bit {
            gc_byte::write_gc_byte::<VM>(
                object,
                gc_byte::read_gc_byte::<VM>(object) | self.header_byte.unlogged_bit,
            );
        }
    }
}

impl<VM: VMBinding> Space<VM> for LargeObjectSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }
    fn init(&mut self, _vm_map: &'static VMMap) {
        let me = unsafe { &*(self as *const Self) };
        self.pr.bind_space(me);
    }

    fn common(&self) -> &CommonSpace<VM> {
        unsafe { &*self.common.get() }
    }

    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        &mut *self.common.get()
    }

    fn release_multiple_pages(&mut self, start: Address) {
        self.pr.release_pages(start);
    }
}

impl<VM: VMBinding> LargeObjectSpace<VM> {
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        constraints: &'static PlanConstraints,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                zeroed,
                vmrequest,
            },
            vm_map,
            mmapper,
            heap,
        );
        LargeObjectSpace {
            pr: if vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common: UnsafeCell::new(common),
            mark_state: 0,
            in_nursery_gc: false,
            treadmill: TreadMill::new(),
            header_byte: HeaderByte::new(constraints),
        }
    }

    pub fn prepare(&mut self, full_heap: bool) {
        if full_heap {
            debug_assert!(self.treadmill.from_space_empty());
            self.mark_state = MARK_BIT - self.mark_state;
        }
        self.treadmill.flip(full_heap);
        self.in_nursery_gc = !full_heap;
    }

    pub fn release(&mut self, full_heap: bool) {
        self.sweep_large_pages(true);
        debug_assert!(self.treadmill.nursery_empty());
        if full_heap {
            self.sweep_large_pages(false);
        }
    }
    // Allow nested-if for this function to make it clear that test_and_mark() is only executed
    // for the outer condition is met.
    #[allow(clippy::collapsible_if)]
    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        let nursery_object = self.is_in_nursery(object);
        if !self.in_nursery_gc || nursery_object {
            // Note that test_and_mark() has side effects
            if self.test_and_mark(object, self.mark_state) {
                let cell = VM::VMObjectModel::object_start_ref(object)
                    - if USE_PRECEEDING_GC_HEADER {
                        PRECEEDING_GC_HEADER_BYTES
                    } else {
                        0
                    };
                self.treadmill.copy(cell, nursery_object);
                trace.process_node(object);
            }
        }
        object
    }

    fn sweep_large_pages(&mut self, sweep_nursery: bool) {
        // FIXME: borrow checker fighting
        // didn't call self.release_multiple_pages
        // so the compiler knows I'm borrowing two different fields
        if sweep_nursery {
            for cell in self.treadmill.collect_nursery() {
                // println!("- cn {}", cell);
                self.pr.release_pages(get_super_page(cell));
            }
        } else {
            for cell in self.treadmill.collect() {
                // println!("- ts {}", cell);
                self.pr.release_pages(get_super_page(cell));
            }
        }
    }

    pub fn allocate_pages(&self, tls: OpaquePointer, pages: usize) -> Address {
        let start = self.acquire(tls, pages);
        if start.is_zero() {
            return start;
        }
        if USE_PRECEEDING_GC_HEADER {
            start + PRECEEDING_GC_HEADER_BYTES
        } else {
            start
        }
    }

    fn test_and_mark(&self, object: ObjectReference, value: u8) -> bool {
        let mask = if self.in_nursery_gc {
            LOS_BIT_MASK
        } else {
            MARK_BIT
        };
        let mut old_value = gc_byte::read_gc_byte::<VM>(object);
        let mut mark_bit = old_value & mask;
        if mark_bit == value {
            return false;
        }
        while !gc_byte::compare_exchange_gc_byte::<VM>(
            object,
            old_value,
            old_value & !LOS_BIT_MASK | value,
        ) {
            old_value = gc_byte::read_gc_byte::<VM>(object);
            mark_bit = old_value & mask;
            if mark_bit == value {
                return false;
            }
        }
        true
    }

    fn test_mark_bit(&self, object: ObjectReference, value: u8) -> bool {
        gc_byte::read_gc_byte::<VM>(object) & MARK_BIT == value
    }

    fn is_in_nursery(&self, object: ObjectReference) -> bool {
        gc_byte::read_gc_byte::<VM>(object) & NURSERY_BIT == NURSERY_BIT
    }
}

fn get_super_page(cell: Address) -> Address {
    cell.align_down(BYTES_IN_PAGE)
}
