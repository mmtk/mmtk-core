use crate::plan::global::CommonPlan;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::ObjectQueue;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::copy::CopySemantics;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::statistics::counter::EventCounter;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::{ObjectModel, VMBinding};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use mmtk_macros::{HasSpaces, PlanTraceObject};

/// Common implementation for generational plans. Each generational plan
/// should include this type, and forward calls to it where possible.
#[derive(HasSpaces, PlanTraceObject)]
pub struct CommonGenPlan<VM: VMBinding> {
    /// The nursery space.
    #[space]
    #[copy_semantics(CopySemantics::PromoteToMature)]
    pub nursery: CopySpace<VM>,
    /// The common plan.
    #[parent]
    pub common: CommonPlan<VM>,
    /// Is this GC full heap?
    pub gc_full_heap: AtomicBool,
    /// Is next GC full heap?
    pub next_gc_full_heap: AtomicBool,
    pub full_heap_gc_count: Arc<Mutex<EventCounter>>,
}

impl<VM: VMBinding> CommonGenPlan<VM> {
    pub fn new(mut args: CreateSpecificPlanArgs<VM>) -> Self {
        let nursery = CopySpace::new(
            args.get_space_args(
                "nursery",
                true,
                VMRequest::fixed_extent(args.global_args.options.get_max_nursery_bytes(), false),
            ),
            true,
        );
        let full_heap_gc_count = args
            .global_args
            .stats
            .new_event_counter("majorGC", true, true);
        let common = CommonPlan::new(args);

        CommonGenPlan {
            nursery,
            common,
            gc_full_heap: AtomicBool::default(),
            next_gc_full_heap: AtomicBool::new(false),
            full_heap_gc_count,
        }
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
    fn virtual_memory_exhausted(plan: &dyn GenerationalPlan<VM = VM>) -> bool {
        ((plan.get_collection_reserved_pages() as f64
            * VM::VMObjectModel::VM_WORST_CASE_COPY_EXPANSION) as usize)
            > plan.get_mature_physical_pages_available()
    }

    /// Check if we need a GC based on the nursery space usage. This method may mark
    /// the following GC as a full heap GC.
    pub fn collection_required<P: Plan<VM = VM>>(
        &self,
        plan: &P,
        space_full: bool,
        space: Option<SpaceStats<VM>>,
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
        if Self::virtual_memory_exhausted(plan.generational().unwrap()) {
            return true;
        }

        // Is the GC triggered by nursery?
        // - if space is none, it is not. Return false immediately.
        // - if space is some, we further check its descriptor.
        let is_triggered_by_nursery = space.map_or(false, |s| {
            s.0.common().descriptor == self.nursery.common().descriptor
        });
        // If space is full and the GC is not triggered by nursery, next GC will be full heap GC.
        if space_full && !is_triggered_by_nursery {
            self.next_gc_full_heap.store(true, Ordering::SeqCst);
        }

        self.common.base.collection_required(plan, space_full)
    }

    pub fn force_full_heap_collection(&self) {
        self.next_gc_full_heap.store(true, Ordering::SeqCst);
    }

    pub fn last_collection_full_heap(&self) -> bool {
        self.gc_full_heap.load(Ordering::Relaxed)
    }

    /// Check if we should do a full heap GC. It returns true if we should have a full heap GC.
    /// It also sets gc_full_heap based on the result.
    pub fn requires_full_heap_collection<P: Plan<VM = VM>>(&self, plan: &P) -> bool {
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
            .global_state
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
                .global_state
                .cur_collection_attempts
                .load(Ordering::SeqCst)
                > 1
        {
            trace!(
                "full heap: next_gc_full_heap = {}, cur_collection_attempts = {}",
                self.next_gc_full_heap.load(Ordering::SeqCst),
                self.common
                    .base
                    .global_state
                    .cur_collection_attempts
                    .load(Ordering::SeqCst)
            );
            // Forces full heap collection
            true
        } else if Self::virtual_memory_exhausted(plan.generational().unwrap()) {
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
    #[allow(unused)] // We now use `PlanTraceObject`, and this mehtod is not used.
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
            "next gc will be full heap? {}, available pages = {}, min nursery = {}",
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
    /// add their own reservation with the value returned by this method.
    pub fn get_collection_reserved_pages(&self) -> usize {
        self.nursery.reserved_pages()
    }

    /// Get pages used by a generational plan. A generational plan should add their own used pages
    /// with the value returned by this method.
    pub fn get_used_pages(&self) -> usize {
        self.nursery.reserved_pages() + self.common.get_used_pages()
    }
}

/// This trait includes methods that are specific to generational plans. This trait needs
/// to be object safe.
pub trait GenerationalPlan: Plan {
    /// Is the current GC a nursery GC? If a GC is not a nursery GC, it will be a full heap GC.
    /// This should only be called during GC.
    fn is_current_gc_nursery(&self) -> bool;

    /// Is the object in the nursery?
    fn is_object_in_nursery(&self, object: ObjectReference) -> bool;

    /// Is the address in the nursery? As we only know addresses rather than object references, the
    /// implementation cannot access per-object metadata. If the plan does not have knowledge whether
    /// the address is in nursery or not (e.g. mature/nursery objects share the same space and are
    /// only differentiated by object metadata), the implementation should return `false` as a more
    /// conservative result.
    fn is_address_in_nursery(&self, addr: Address) -> bool;

    /// Return the number of pages available for allocation into the mature space.
    fn get_mature_physical_pages_available(&self) -> usize;

    /// Return the number of used pages in the mature space.
    fn get_mature_reserved_pages(&self) -> usize;

    /// Return whether last GC is a full GC.
    fn last_collection_full_heap(&self) -> bool;

    /// Force the next collection to be full heap.
    fn force_full_heap_collection(&self);
}

/// This trait is the extension trait for [`GenerationalPlan`] (see Rust's extension trait pattern).
/// Generally any method should be put to [`GenerationalPlan`] if possible while keeping [`GenerationalPlan`]
/// object safe. In this case, generic methods will be put to this extension trait.
pub trait GenerationalPlanExt<VM: VMBinding>: GenerationalPlan<VM = VM> {
    /// Trace an object in nursery collection. If the object is in nursery, we should call `trace_object`
    /// on the space. Otherwise, we can just return the object.
    fn trace_object_nursery<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;
}

/// Is current GC only collecting objects allocated since last GC? This method can be called
/// with any plan (generational or not). For non generational plans, it will always return false.
pub fn is_nursery_gc<VM: VMBinding>(plan: &dyn Plan<VM = VM>) -> bool {
    plan.generational()
        .map_or(false, |plan| plan.is_current_gc_nursery())
}
