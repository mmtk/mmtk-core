use crate::Plan;
use crate::plan::GcStatus;
use crate::plan::generational::global::GenerationalPlan;
use crate::plan::immix;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::copy::CopyConfig;
use crate::util::copy::CopySelector;
use crate::util::heap::HeapMeta;
use crate::util::metadata::MetadataSpec;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::vm::VMBinding;
use crate::plan::global::CommonPlan;
use crate::policy::immix::ImmixSpace;
use crate::plan::PlanConstraints;
use crate::util::copy::CopySemantics;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::options::Options;
use crate::policy::sft::SFT;
use crate::vm::ObjectModel;
use crate::plan::global::CreateGeneralPlanArgs;

use atomic::Ordering;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use mmtk_macros::PlanTraceObject;

use super::gc_work::StickyImmixNurseryGCWorkContext;

#[derive(PlanTraceObject)]
pub struct StickyImmix<VM: VMBinding> {
    #[fallback_trace]
    pub(in crate::plan::sticky::immix) immix: immix::Immix<VM>,
    gc_full_heap: AtomicBool,
    next_gc_full_heap: AtomicBool,
}

pub const STICKY_IMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    ..immix::IMMIX_CONSTRAINTS
};

impl<VM: VMBinding> crate::plan::generational::global::HasNursery<VM> for StickyImmix<VM> {
    fn is_object_in_nursery(&self, object: crate::util::ObjectReference) -> bool {
        self.immix.immix_space.in_space(object) && !self.immix.immix_space.is_marked_with_current_mark_state(object)
    }

    fn is_address_in_nursery(&self, addr: crate::util::Address) -> bool {
        false
    }

    fn trace_object_nursery<Q: crate::ObjectQueue>(
        &self,
        queue: &mut Q,
        object: crate::util::ObjectReference,
        worker: &mut crate::scheduler::GCWorker<VM>,
    ) -> crate::util::ObjectReference {
        if self.immix.immix_space.in_space(object) {
            if !self.is_object_in_nursery(object) {
                // Mature object
                return object;
            } else if crate::policy::immix::PREFER_COPY_ON_NURSERY_GC {
                // Should we use DefaultCopy? Or PromoteMature?
                return self.immix.immix_space.trace_object_with_opportunistic_copy(queue, object, CopySemantics::DefaultCopy, worker, true);
            } else {
                return self.immix.immix_space.trace_object_without_moving(queue, object);
            }
        }

        if self.immix.common().get_los().in_space(object) {
            return self.immix.common().get_los().trace_object::<Q>(queue, object);
        }

        object
    }
}

impl<VM: VMBinding> Plan for StickyImmix<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static crate::plan::PlanConstraints {
        &STICKY_IMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix.immix_space)],
            constraints: &STICKY_IMMIX_CONSTRAINTS,
        }
    }

    fn base(&self) -> &crate::plan::global::BasePlan<Self::VM> {
        self.immix.base()
    }

    fn common(&self) -> &CommonPlan<Self::VM> {
        self.immix.common()
    }

    fn force_full_heap_collection(&self) {
        self.next_gc_full_heap.store(true, Ordering::SeqCst);
    }

    fn last_collection_full_heap(&self) -> bool {
        self.gc_full_heap.load(Ordering::SeqCst)
    }

    fn schedule_collection(&'static self, scheduler: &crate::scheduler::GCWorkScheduler<Self::VM>) {
        let is_full_heap = self.requires_full_heap_collection();
        self.gc_full_heap.store(is_full_heap, Ordering::SeqCst);

        if !is_full_heap {
            info!("Nursery GC");
            self.base().set_collection_kind::<Self>(self);
            self.base().set_gc_status(GcStatus::GcPrepare);
            // nursery GC -- we schedule it
            scheduler.schedule_common_work::<StickyImmixNurseryGCWorkContext<VM>>(self);
        } else {
            self.immix.schedule_collection(scheduler);
        }
    }

    fn get_spaces(&self) -> Vec<&dyn crate::policy::space::Space<Self::VM>> {
        self.immix.get_spaces()
    }

    fn get_allocator_mapping(&self) -> &'static enum_map::EnumMap<crate::AllocationSemantics, crate::util::alloc::AllocatorSelector> {
        &super::mutator::ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: crate::util::VMWorkerThread) {
        if self.is_current_gc_nursery() {
            self.immix.immix_space.prepare(false);
        } else {
            self.immix.prepare(tls);
        }
    }

    fn release(&mut self, tls: crate::util::VMWorkerThread) {
        if self.is_current_gc_nursery() {
            let was_defrag = self.immix.immix_space.release(false);
            self.immix.last_gc_was_defrag.store(was_defrag, Ordering::Relaxed);
            return;
        } else {
            self.immix.release(tls);
            self.next_gc_full_heap.store(self.get_available_pages() < self.options().get_min_nursery_pages(), Ordering::Relaxed);
        }
    }

    fn collection_required(&self, space_full: bool, space: Option<&dyn crate::policy::space::Space<Self::VM>>) -> bool {
        let nursery_full = self.immix.immix_space.get_pages_allocated() > self.options().get_max_nursery_pages();
        if space_full && space.is_some() && space.unwrap().name() == self.immix.immix_space.name() {
            self.next_gc_full_heap.store(true, Ordering::SeqCst);
        }
        return self.immix.collection_required(space_full, space) || nursery_full
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.immix.get_collection_reserved_pages() + self.immix.immix_space.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.immix.get_used_pages()
    }
}

impl<VM: VMBinding> StickyImmix<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        Self {
            immix: immix::Immix::new(args),
            gc_full_heap: AtomicBool::new(false),
            next_gc_full_heap: AtomicBool::new(false),
        }
    }

    fn is_current_gc_nursery(&self) -> bool {
        !self.gc_full_heap.load(Ordering::SeqCst)
    }

    fn requires_full_heap_collection(&self) -> bool {
        if self.immix
            .common
            .base
            .user_triggered_collection
            .load(Ordering::SeqCst)
            && *self.immix.common.base.options.full_heap_system_gc
        {
            // User triggered collection, and we force full heap for user triggered collection
            true
        } else if self.next_gc_full_heap.load(Ordering::SeqCst)
            || self.immix
                .common
                .base
                .cur_collection_attempts
                .load(Ordering::SeqCst)
                > 1
        {
            // Forces full heap collection
            true
        } else {
            false
        }
    }
}