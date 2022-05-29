use super::gc_work::GenCopyGCWorkContext;
use super::gc_work::GenCopyNurseryGCWorkContext;
use super::mutator::ALLOCATOR_MAPPING;
use crate::plan::generational::global::Gen;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::VMWorkerThread;
use crate::vm::*;
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mmtk_macros::PlanTraceObject;

#[derive(PlanTraceObject)]
pub struct GenCopy<VM: VMBinding> {
    #[fallback_trace]
    pub gen: Gen<VM>,
    pub hi: AtomicBool,
    #[trace(CopySemantics::Mature)]
    pub copyspace0: CopySpace<VM>,
    #[trace(CopySemantics::Mature)]
    pub copyspace1: CopySpace<VM>,
}

pub const GENCOPY_CONSTRAINTS: PlanConstraints = crate::plan::generational::GEN_CONSTRAINTS;

impl<VM: VMBinding> Plan for GenCopy<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &GENCOPY_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::Mature => CopySelector::CopySpace(0),
                CopySemantics::PromoteToMature => CopySelector::CopySpace(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![
                // The tospace argument doesn't matter, we will rebind before a GC anyway.
                (CopySelector::CopySpace(0), self.tospace()),
            ],
            constraints: &GENCOPY_CONSTRAINTS,
        }
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool
    where
        Self: Sized,
    {
        self.gen.collection_required(self, space_full, space)
    }

    fn force_full_heap_collection(&self) {
        self.gen.force_full_heap_collection()
    }

    fn last_collection_full_heap(&self) -> bool {
        self.gen.last_collection_full_heap()
    }

    fn gc_init(&mut self, heap_size: usize, vm_map: &'static VMMap) {
        self.gen.gc_init(heap_size, vm_map);
        self.copyspace0.init(vm_map);
        self.copyspace1.init(vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        let is_full_heap = self.request_full_heap_collection();
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        if is_full_heap {
            scheduler.schedule_common_work::<GenCopyGCWorkContext<VM>>(self);
        } else {
            scheduler.schedule_common_work::<GenCopyNurseryGCWorkContext<VM>>(self);
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.gen.prepare(tls);
        if full_heap {
            self.hi
                .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
        }
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);

        self.fromspace_mut()
            .set_copy_for_sft_trace(Some(CopySemantics::Mature));
        self.tospace_mut().set_copy_for_sft_trace(None);
    }

    fn prepare_worker(&self, worker: &mut GCWorker<Self::VM>) {
        unsafe { worker.get_copy_context_mut().copy[0].assume_init_mut() }.rebind(self.tospace());
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.gen.release(tls);
        if full_heap {
            self.fromspace().release();
        }

        self.gen
            .set_next_gc_full_heap(Gen::should_next_gc_be_full_heap(self));
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.gen.get_collection_reserved_pages() + self.tospace().reserved_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.gen.get_used_pages() + self.tospace().reserved_pages()
    }

    /// Return the number of pages avilable for allocation. Assuming all future allocations goes to nursery.
    fn get_available_pages(&self) -> usize {
        // super.get_pages_avail() / 2 to reserve pages for copying
        (self
            .get_total_pages()
            .saturating_sub(self.get_reserved_pages()))
            >> 1
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.gen.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.gen.common
    }

    fn generational(&self) -> &Gen<VM> {
        &self.gen
    }

    fn is_current_gc_nursery(&self) -> bool {
        !self.gen.gc_full_heap.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> GenCopy<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        // We have no specific side metadata for copying. So just use the ones from generational.
        let global_metadata_specs =
            crate::plan::generational::new_generational_global_metadata_specs::<VM>();

        let copyspace0 = CopySpace::new(
            "copyspace0",
            false,
            true,
            VMRequest::discontiguous(),
            global_metadata_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
        );
        let copyspace1 = CopySpace::new(
            "copyspace1",
            true,
            true,
            VMRequest::discontiguous(),
            global_metadata_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
        );

        let res = GenCopy {
            gen: Gen::new(
                heap,
                global_metadata_specs,
                &GENCOPY_CONSTRAINTS,
                vm_map,
                mmapper,
                options,
            ),
            hi: AtomicBool::new(false),
            copyspace0,
            copyspace1,
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        {
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            res.gen
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.copyspace0
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.copyspace1
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        res
    }

    fn request_full_heap_collection(&self) -> bool {
        self.gen
            .request_full_heap_collection(self.get_total_pages(), self.get_reserved_pages())
    }

    pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn tospace_mut(&mut self) -> &mut CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &mut self.copyspace1
        } else {
            &mut self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }

    pub fn fromspace_mut(&mut self) -> &mut CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &mut self.copyspace0
        } else {
            &mut self.copyspace1
        }
    }
}
