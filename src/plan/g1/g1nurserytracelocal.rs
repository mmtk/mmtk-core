use plan::{TraceLocal, TransitiveClosure};
use plan::g1::PLAN;
use plan::trace::Trace;
use policy::space::Space;
use util::{Address, ObjectReference};
use util::queue::LocalQueue;
use vm::*;
use libc::c_void;
use super::g1;
use policy::region::*;
use ::util::heap::layout::Mmapper;
use ::util::heap::layout::heap_layout::MMAPPER;

pub struct G1NurseryTraceLocal {
    tls: *mut c_void,
    values: LocalQueue<'static, ObjectReference>,
    root_locations: LocalQueue<'static, Address>,
    bytes_copied: usize,
}

impl TransitiveClosure for G1NurseryTraceLocal {
    fn process_edge(&mut self, src: ObjectReference, slot: Address) {
        debug_assert!(MMAPPER.address_is_mapped(slot));
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            if RegionSpace::is_cross_region_ref(src, slot, new_object) && PLAN.region_space.in_space(new_object) {
                Region::of_object(new_object).remset().add_card(Card::of(src))
            }
            unsafe { slot.store(new_object) };
        }
    }

    fn process_node(&mut self, object: ObjectReference) {
        self.values.enqueue(object);
    }
}

impl TraceLocal for G1NurseryTraceLocal {
    fn process_remembered_sets(&mut self) {
    }

    fn overwrite_reference_during_trace(&self) -> bool {
        true
    }

    fn process_roots(&mut self) {
        while let Some(slot) = self.root_locations.dequeue() {
            self.process_root_edge(slot, true)
        }
        debug_assert!(self.root_locations.is_empty());
    }

    fn process_root_edge(&mut self, slot: Address, _untraced: bool) {
        let object: ObjectReference = unsafe { slot.load() };
        let new_object = self.trace_object(object);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_object) };
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        debug_assert!(super::ENABLE_REMEMBERED_SETS);
        let tls = self.tls;

        if object.is_null() {
            object
        } else if PLAN.region_space.in_space(object) {
            let region = Region::of_object(object);
            debug_assert!(region.committed);
            if region.relocate {
                // println!("Nursery Eva start {:?} {:?}", object, region);
                let o = if region.prev_mark_table().is_marked(object) {
                    let allocator = Self::pick_copy_allocator(object);
                    // println!("Nursery Copy start {:?} {:?}", object, region);
                    let (o, s) = PLAN.region_space.trace_evacuate_object_in_cset(self, object, allocator, tls);
                    self.bytes_copied += s;
                    o
                } else {
                    ObjectReference::null()
                };
                // println!("Nursery Eva end");
                o
            } else {
                object
            }
        } else {
            debug_assert!(PLAN.is_mapped_object(object));
            object
        }
    }

    fn complete_trace(&mut self) {
        let start = ::std::time::SystemTime::now();
        self.bytes_copied = 0;
        let id = self.tls;
        self.process_roots();
        debug_assert!(self.root_locations.is_empty());
        loop {
            while let Some(object) = self.values.dequeue() {
                VMScanning::scan_object(self, object, id);
            }
            self.process_remembered_sets();
            if self.values.is_empty() {
                break;
            }
        }
        let time = start.elapsed().unwrap().as_millis() as usize;
        PLAN.predictor.timer.report_evacuation_time(time, self.bytes_copied);
        debug_assert!(self.root_locations.is_empty());
        debug_assert!(self.values.is_empty());
    }

    fn release(&mut self) {
        self.root_locations.reset();
        self.values.reset();
    }

    fn process_interior_edge(&mut self, target: ObjectReference, slot: Address, _root: bool) {
        let interior_ref: Address = unsafe { slot.load() };
        let offset = interior_ref - target.to_address();
        let new_target = self.trace_object(target);
        if self.overwrite_reference_during_trace() {
            unsafe { slot.store(new_target.to_address() + offset) };
        }
    }

    fn report_delayed_root_edge(&mut self, slot: Address) {
        self.root_locations.enqueue(slot);
    }

    fn will_not_move_in_current_collection(&self, obj: ObjectReference) -> bool {
        if PLAN.region_space.in_space(obj) {
            false
        } else {
            true
        }
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        } else if PLAN.region_space.in_space(object) {
            if Region::of_object(object).relocate {
                ::util::forwarding_word::is_forwarded_or_being_forwarded(object)
            } else {
                true
            }
        } else {
            debug_assert!(PLAN.is_mapped_object(object));
            true
        }
    }
}

impl G1NurseryTraceLocal {
    pub fn new(trace: &'static Trace) -> Self {
        Self {
            tls: 0 as *mut c_void,
            values: trace.values.spawn_local(),
            root_locations: trace.root_locations.spawn_local(),
            bytes_copied: 0,
        }
    }

    pub fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
    }

    pub fn is_empty(&self) -> bool {
        self.root_locations.is_empty() && self.values.is_empty()
    }
    
    pub fn flush(&mut self) {
        self.values.flush();
        self.root_locations.flush();
    }

    #[inline]
    fn pick_copy_allocator(o: ObjectReference) -> ::plan::Allocator {
        debug_assert!(super::ENABLE_GENERATIONAL_GC);
        match Region::of_object(o).generation {
            Gen::Eden => g1::ALLOC_SURVIVOR,
            _ => g1::ALLOC_OLD,
        }
    }
}
