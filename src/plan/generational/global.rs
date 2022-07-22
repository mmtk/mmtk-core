use crate::plan::global::CommonPlan;
use crate::plan::ObjectQueue;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::conversions;
use crate::util::copy::CopySemantics;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::options::Options;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use mmtk_macros::PlanTraceObject;

/// Common implementation for generational plans. Each generational plan
/// should include this type, and forward calls to it where possible.
#[derive(PlanTraceObject)]
pub struct Gen<VM: VMBinding> {
    /// The nursery space.
    #[trace(CopySemantics::PromoteToMature)]
    pub nursery: CopySpace<VM>,
    /// The common plan.
    #[fallback_trace]
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
        options: Arc<Options>,
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

    /// Verify side metadata specs used in the spaces in Gen.
    pub fn verify_side_metadata_sanity(&self, sanity: &mut SideMetadataSanity) {
        self.common.verify_side_metadata_sanity(sanity);
        self.nursery.verify_side_metadata_sanity(sanity);
    }

    /// Get spaces in generation plans
    pub fn get_spaces(&self) -> Vec<&dyn Space<VM>> {
        let mut ret = self.common.get_spaces();
        ret.push(&self.nursery);
        ret
    }

    /// Prepare Gen. This should be called by a single thread in GC prepare work.
    pub fn prepare(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.common.prepare(tls, full_heap);
        self.nursery.prepare(true);
        self.nursery
            .set_copy_for_sft_trace(Some(CopySemantics::PromoteToMature));
    }

    /// Release Gen. This should be called by a single thread in GC release work.
    pub fn release(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.common.release(tls, full_heap);
        self.nursery.release();
    }

    /// Check if we need a GC based on the nursery space usage. This method may mark
    /// the following GC as a full heap GC.
    pub fn collection_required<P: Plan>(
        &self,
        plan: &P,
        space_full: bool,
        space: Option<&dyn Space<VM>>,
    ) -> bool {
        let nursery_full = self.nursery.reserved_pages()
            >= (conversions::bytes_to_pages_up(*self.common.base.options.max_nursery));
        if nursery_full {
            return true;
        }

        // Is the GC triggered by nursery?
        // - if space is none, it is not. Return false immediately.
        // - if space is some, we further check its descriptor.
        let is_triggered_by_nursery = space.map_or(false, |s| {
            s.common().descriptor == self.nursery.common().descriptor
        });
        // If space is full and the GC is not triggered by nursery, next GC will be full heap GC.
        if space_full && !is_triggered_by_nursery {
            self.next_gc_full_heap.store(true, Ordering::SeqCst);
        }

        self.common.base.collection_required(plan, space_full)
    }

    pub fn force_full_heap_collection(&self) {
        self.next_gc_full_heap.store(true, Ordering::Relaxed);
    }

    pub fn last_collection_full_heap(&self) -> bool {
        self.gc_full_heap.load(Ordering::Relaxed)
    }

    /// Check if we should do a full heap GC. It returns true if we should have a full heap GC.
    /// It also sets gc_full_heap based on the result.
    pub fn request_full_heap_collection(&self, total_pages: usize, reserved_pages: usize) -> bool {
        // Allow the same 'true' block for if-else.
        // The conditions are complex, and it is easier to read if we put them to separate if blocks.
        #[allow(clippy::if_same_then_else)]
        let is_full_heap = if crate::plan::generational::FULL_NURSERY_GC {
            // For barrier overhead measurements, we always do full gc in nursery collections.
            true
        } else if self
            .common
            .base
            .user_triggered_collection
            .load(Ordering::SeqCst)
            && *self.common.base.options.full_heap_system_gc
        {
            // User triggered collection, and we force full heap for user triggered collection
            true
        } else if self.next_gc_full_heap.load(Ordering::SeqCst)
            || self
                .common
                .base
                .cur_collection_attempts
                .load(Ordering::SeqCst)
                > 1
        {
            // Forces full heap collection
            true
        } else {
            total_pages <= reserved_pages
        };

        self.gc_full_heap.store(is_full_heap, Ordering::SeqCst);

        info!(
            "{}",
            if is_full_heap {
                "Full heap GC"
            } else {
                "nursery GC"
            }
        );

        is_full_heap
    }

    /// Trace objects for spaces in generational and common plans for a full heap GC.
    pub fn trace_object_full_heap<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        if self.nursery.in_space(object) {
            return self.nursery.trace_object::<Q>(
                queue,
                object,
                Some(CopySemantics::PromoteToMature),
                worker,
            );
        }
        self.common.trace_object::<Q>(queue, object, worker)
    }

    /// Trace objects for spaces in generational and common plans for a nursery GC.
    pub fn trace_object_nursery<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        // Evacuate nursery objects
        if self.nursery.in_space(object) {
            return self.nursery.trace_object::<Q>(
                queue,
                object,
                Some(CopySemantics::PromoteToMature),
                worker,
            );
        }
        // We may alloc large object into LOS as nursery objects. Trace them here.
        if self.common.get_los().in_space(object) {
            return self.common.get_los().trace_object::<Q>(queue, object);
        }
        object
    }

    /// Is the current GC a nursery GC?
    pub fn is_current_gc_nursery(&self) -> bool {
        !self.gc_full_heap.load(Ordering::SeqCst)
    }

    /// Check a plan to see if the next GC should be a full heap GC.
    pub fn should_next_gc_be_full_heap(plan: &dyn Plan<VM = VM>) -> bool {
        plan.get_available_pages()
            < conversions::bytes_to_pages_up(*plan.base().options.min_nursery)
    }

    /// Set next_gc_full_heap to the given value.
    pub fn set_next_gc_full_heap(&self, next_gc_full_heap: bool) {
        self.next_gc_full_heap
            .store(next_gc_full_heap, Ordering::SeqCst);
    }

    /// Get pages reserved for the collection by a generational plan. A generational plan should
    /// add their own reservatioin with the value returned by this method.
    pub fn get_collection_reserved_pages(&self) -> usize {
        self.nursery.reserved_pages()
    }

    /// Get pages used by a generational plan. A generational plan should add their own used pages
    /// with the value returned by this method.
    pub fn get_used_pages(&self) -> usize {
        self.nursery.reserved_pages() + self.common.get_used_pages()
    }
}
