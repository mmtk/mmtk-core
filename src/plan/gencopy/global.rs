use crate::policy::space::Space;

use super::gc_works::{
    GenCopyCopyContext, GenCopyMatureProcessEdges, GenCopyNurseryProcessEdges, SanityGCProcessEdges,
};
use super::mutator::create_gencopy_mutator;
use super::mutator::ALLOCATOR_MAPPING;
use crate::mmtk::MMTK;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::Allocator;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::OpaquePointer;
use crate::vm::*;
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub type SelectedPlan<VM> = GenCopy<VM>;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const NURSERY_SIZE: usize = 16 * 1024 * 1024;

pub struct GenCopy<VM: VMBinding> {
    pub nursery: CopySpace<VM>,
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
    in_nursery: AtomicBool,
    in_sanity: AtomicBool,
    pub scheduler: &'static MMTkScheduler<VM>,
}

unsafe impl<VM: VMBinding> Sync for GenCopy<VM> {}

impl<VM: VMBinding> Plan for GenCopy<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = GenCopyCopyContext<VM>;

    fn collection_required(&self, space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
        Self: Sized,
    {
        let nursery_full = self.nursery.reserved_pages() >= (NURSERY_SIZE / 4096);
        let heap_full = self.get_pages_reserved() > self.get_total_pages();
        space_full || nursery_full || heap_full
    }

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        GenCopy {
            nursery: CopySpace::new(
                "nursery",
                false,
                true,
                VMRequest::fixed_extent(NURSERY_SIZE, false),
                vm_map,
                mmapper,
                &mut heap,
            ),
            hi: AtomicBool::new(false),
            copyspace0: CopySpace::new(
                "copyspace0",
                false,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            copyspace1: CopySpace::new(
                "copyspace1",
                true,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
            in_nursery: AtomicBool::default(),
            in_sanity: AtomicBool::default(),
            scheduler,
        }
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.nursery.init(&vm_map);
        self.copyspace0.init(&vm_map);
        self.copyspace1.init(&vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        let in_nursery = !self.request_full_heap_collection();
        self.in_nursery.store(in_nursery, Ordering::SeqCst);
        self.in_sanity.store(false, Ordering::SeqCst);
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);

        // Stop & scan mutators (mutator scanning can happen before STW)
        if in_nursery {
            scheduler
                .unconstrained_works
                .add(StopMutators::<GenCopyNurseryProcessEdges<VM>>::new());
        } else {
            scheduler
                .unconstrained_works
                .add(StopMutators::<GenCopyMatureProcessEdges<VM>>::new());
        }
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        // Resume mutators
        if cfg!(feature = "gencopy_sanity_gc") {
            scheduler.final_stage.add(ScheduleSanityGC);
        }
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn schedule_sanity_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        println!("sanity gc");
        self.in_sanity.store(true, Ordering::SeqCst);

        // Stop & scan mutators (mutator scanning can happen before STW)
        for mutator in <VM as VMBinding>::VMActivePlan::mutators() {
            scheduler
                .prepare_stage
                .add(ScanStackRoot::<SanityGCProcessEdges<VM>>(mutator));
        }
        scheduler
            .prepare_stage
            .add(ScanVMSpecificRoots::<SanityGCProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
    }

    fn bind_mutator(
        &'static self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_gencopy_mutator(tls, mmtk))
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<Allocator, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, tls: OpaquePointer) {
        // self.fromspace().unprotect();
        if !self.in_sanity() {
            self.common.prepare(tls, true);
            self.nursery.prepare(true);
            if !self.in_nursery() {
                self.hi
                    .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
            }
            let hi = self.hi.load(Ordering::SeqCst);
            self.copyspace0.prepare(hi);
            self.copyspace1.prepare(!hi);
        } else {
            self.common.prepare(tls, true);
            self.nursery.sanity_prepare();
            self.copyspace0.sanity_prepare();
            self.copyspace1.sanity_prepare();
        }
    }

    fn release(&self, tls: OpaquePointer) {
        if !self.in_sanity() {
            self.common.release(tls, true);
            self.nursery.release();
            if !self.in_nursery() {
                self.fromspace().release();
            }
        } else {
            self.common.release(tls, true);
            self.nursery.sanity_release();
            if !self.in_nursery() {
                self.fromspace().sanity_release();
            }
            // self.copyspace1.sanity_release();
        }
    }

    fn get_collection_reserve(&self) -> usize {
        self.nursery.reserved_pages() + self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.nursery.reserved_pages()
            + self.tospace().reserved_pages()
            + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn in_nursery(&self) -> bool {
        self.in_nursery.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> GenCopy<VM> {
    fn request_full_heap_collection(&self) -> bool {
        self.get_total_pages() <= self.get_pages_reserved()
    }

    pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }

    pub fn in_sanity(&self) -> bool {
        self.in_sanity.load(Ordering::SeqCst)
    }
}
