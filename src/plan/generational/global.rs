use crate::plan::global::CommonPlan;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::ObjectQueue;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::copy::CopySemantics;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::statistics::counter::EventCounter;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::{ObjectModel, VMBinding};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

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
    pub full_heap_gc_count: Arc<Mutex<EventCounter>>,
}

impl<VM: VMBinding> Gen<VM> {
    pub fn new(mut args: CreateSpecificPlanArgs<VM>) -> Self {
        let nursery = CopySpace::new(
            args.get_space_args(
                "nursery",
                true,
                VMRequest::fixed_extent(args.global_args.options.get_max_nursery_bytes(), false),
            ),
            true,
        );
        let common = CommonPlan::new(args);

        let full_heap_gc_count = common.base.stats.new_event_counter("majorGC", true, true);

        Gen {
            nursery,
            common,
            gc_full_heap: AtomicBool::default(),
            next_gc_full_heap: AtomicBool::new(false),
            full_heap_gc_count,
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
        if full_heap {
            self.full_heap_gc_count.lock().unwrap().inc();
        }
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

    /// Independent of how many pages remain in the page budget (a function of heap size), we must
    /// ensure we never exhaust virtual memory. Therefore we must never let the nursery grow to the
    /// extent that it can't be copied into the mature space.
    ///
    /// Returns `true` if the nursery has grown to the extent that it may not be able to be copied
    /// into the mature space.
    fn virtual_memory_exhausted<P: Plan>(&self, plan: &P) -> bool {
        ((plan.get_collection_reserved_pages() as f64
            * VM::VMObjectModel::VM_WORST_CASE_COPY_EXPANSION) as usize)
            > plan.get_mature_physical_pages_available()
    }

    /// Check if we need a GC based on the nursery space usage. This method may mark
    /// the following GC as a full heap GC.
    pub fn collection_required<P: Plan>(
        &self,
        plan: &P,
        space_full: bool,
        space: Option<&dyn Space<VM>>,
    ) -> bool {
        let cur_nursery = self.nursery.reserved_pages();
        let max_nursery = self.common.base.options.get_max_nursery_pages();
        let nursery_full = cur_nursery >= max_nursery;
        trace!(
            "nursery_full = {:?} (nursery = {}, max_nursery = {})",
            nursery_full,
            cur_nursery,
            max_nursery,
        );

        if nursery_full {
            return true;
        }

        if self.virtual_memory_exhausted(plan) {
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
    pub fn requires_full_heap_collection<P: Plan>(&self, plan: &P) -> bool {
        // Allow the same 'true' block for if-else.
        // The conditions are complex, and it is easier to read if we put them to separate if blocks.
        #[allow(clippy::if_same_then_else, clippy::needless_bool)]
        let is_full_heap = if crate::plan::generational::FULL_NURSERY_GC {
            trace!("full heap: forced full heap");
            // For barrier overhead measurements, we always do full gc in nursery collections.
            true
        } else if self
            .common
            .base
            .user_triggered_collection
            .load(Ordering::SeqCst)
            && *self.common.base.options.full_heap_system_gc
        {
            trace!("full heap: user triggered");
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
            trace!(
                "full heap: next_gc_full_heap = {}, cur_collection_attempts = {}",
                self.next_gc_full_heap.load(Ordering::SeqCst),
                self.common
                    .base
                    .cur_collection_attempts
                    .load(Ordering::SeqCst)
            );
            // Forces full heap collection
            true
        } else if self.virtual_memory_exhausted(plan) {
            trace!("full heap: virtual memory exhausted");
            true
        } else {
            // We use an Appel-style nursery. The default GC (even for a "heap-full" collection)
            // for generational GCs should be a nursery GC. A full-heap GC should only happen if
            // there is not enough memory available for allocating into the nursery (i.e. the
            // available pages in the nursery are less than the minimum nursery pages), if the
            // virtual memory has been exhausted, or if it is an emergency GC.
            false
        };

        self.gc_full_heap.store(is_full_heap, Ordering::SeqCst);

        info!(
            "{}",
            if is_full_heap {
                "Full heap GC"
            } else {
                "Nursery GC"
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
    ///
    /// Note that this function should be called after all spaces have been released. This is
    /// required as we may get incorrect values since this function uses
    /// [`get_available_pages`](crate::plan::Plan::get_available_pages)
    /// whose value depends on which spaces have been released.
    pub fn should_next_gc_be_full_heap(plan: &dyn Plan<VM = VM>) -> bool {
        let available = plan.get_available_pages();
        let min_nursery = plan.base().options.get_min_nursery_pages();
        let next_gc_full_heap = available < min_nursery;
        trace!(
            "next gc will be full heap? {}, availabe pages = {}, min nursery = {}",
            next_gc_full_heap,
            available,
            min_nursery
        );
        next_gc_full_heap
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
