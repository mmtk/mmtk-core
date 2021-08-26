use crate::vm::VMBinding;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::plan::global::CommonPlan;
use crate::plan::PlanConstraints;
use crate::util::heap::HeapMeta;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::heap::VMRequest;
use crate::scheduler::*;
use crate::util::VMWorkerThread;
use crate::util::conversions;
use crate::plan::Plan;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::plan::TransitiveClosure;
use crate::plan::CopyContext;
use crate::util::ObjectReference;
use crate::plan::AllocationSemantics;

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct Gen<VM: VMBinding> {
    pub nursery: CopySpace<VM>,
    pub common: CommonPlan<VM>,
    /// Is this GC full heap?
    pub gc_full_heap: AtomicBool,
    /// Is next GC full heap?
    pub next_gc_full_heap: AtomicBool,
}

impl<VM: VMBinding> Gen<VM> {
    pub fn new(
        mut heap: HeapMeta,
        global_metadata_specs: Vec<SideMetadataSpec>,
        constraints: &'static PlanConstraints,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        Gen {
            nursery: CopySpace::new(
                "nursery",
                false,
                true,
                VMRequest::fixed_extent(crate::util::options::NURSERY_SIZE, false),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                constraints,
                global_metadata_specs,
            ),
            gc_full_heap: AtomicBool::default(),
            next_gc_full_heap: AtomicBool::new(false),
        }
    }

    pub fn verify_side_metadata_sanity(&self, sanity: &mut SideMetadataSanity) {
        self.common.verify_side_metadata_sanity(sanity);
        self.nursery.verify_side_metadata_sanity(sanity);
    }

    pub fn gc_init(&mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<GCWorkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.nursery.init(vm_map);
    }

    pub fn prepare(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.common.prepare(tls, full_heap);
        self.nursery.prepare(true);
    }

    pub fn release(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.common.release(tls, full_heap);
        self.nursery.release();
    }

    pub fn collection_required<P: Plan>(&self, plan: &P, space_full: bool, space: &dyn Space<VM>) -> bool {
        let nursery_full = self.nursery.reserved_pages() >= (conversions::bytes_to_pages_up(self.common.base.options.max_nursery));
        if nursery_full {
            return true;
        }

        if space_full && space.common().descriptor != self.nursery.common().descriptor {
            self.next_gc_full_heap.store(true, Ordering::SeqCst);
        }

        self.common.base.collection_required(plan, space_full, space)
    }

    pub fn request_full_heap_collection(&self, used_pages: usize, reserved_pages: usize) -> bool {
        // For barrier overhead measurements, we always do full gc in nursery collections.
        if crate::plan::generational::copying::FULL_NURSERY_GC {
            return true;
        }

        if self.common.base.user_triggered_collection.load(Ordering::SeqCst)
            && self.common.base.options.full_heap_system_gc
        {
            return true;
        }

        if self.next_gc_full_heap.load(Ordering::SeqCst)
            || self.common.base.cur_collection_attempts.load(Ordering::SeqCst) > 1
        {
            // Forces full heap collection
            return true;
        }

        let is_full_heap = used_pages <= reserved_pages;
        if is_full_heap {
            self.gc_full_heap.store(is_full_heap, Ordering::SeqCst);
        }

        is_full_heap
    }

    pub fn trace_object_full_heap<T: TransitiveClosure, C: CopyContext + GCWorkerLocal>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        copy_context: &mut C,
    ) -> ObjectReference {
        if self.nursery.in_space(object) {
            return self
                .nursery
                .trace_object::<T, C>(
                    trace,
                    object,
                    AllocationSemantics::Default,
                    copy_context,
                );
        }
        self.common.trace_object::<T, C>(trace, object)
    }

    pub fn trace_object_nursery<T: TransitiveClosure, C: CopyContext + GCWorkerLocal>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        copy_context: &mut C,
    ) -> ObjectReference {
        // Evacuate nursery objects
        if self.nursery.in_space(object) {
            return self
                .nursery
                .trace_object::<T, C>(
                    trace,
                    object,
                    crate::plan::global::AllocationSemantics::Default,
                    copy_context,
                );
        }
        // We may alloc large object into LOS as nursery objects. Trace them here.
        if self.common.get_los().in_space(object) {
            return self
                .common
                .get_los()
                .trace_object::<T>(trace, object);
        }
        object
    }

    pub fn is_current_gc_nursery(&self) -> bool {
        !self.gc_full_heap.load(Ordering::SeqCst)
    }

    pub fn should_next_gc_be_full_heap(plan: &dyn Plan<VM=VM>) -> bool {
        plan.get_pages_avail()
            < conversions::bytes_to_pages_up(plan.base().options.min_nursery)
    }

    pub fn set_next_gc_full_heap(&self, next_gc_full_heap: bool) {
        self.next_gc_full_heap.store(next_gc_full_heap, Ordering::SeqCst);
    }

    pub fn get_collection_reserve(&self) -> usize {
        self.nursery.reserved_pages()
    }

    pub fn get_pages_used(&self) -> usize {
        self.nursery.reserved_pages() + self.common.get_pages_used()
    }
}