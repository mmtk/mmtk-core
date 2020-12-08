use super::gc_works::{MGCCopyContext, MGCMatureProcessEdges, MGCNurseryProcessEdges};
use crate::{mmtk::MMTK, util::constants::LOG_BYTES_IN_PAGE};
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::mygencopy::mutator::create_mgc_mutator;
use crate::plan::mygencopy::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::sync::{Arc, atomic::AtomicBool};

use atomic::Ordering;
use enum_map::EnumMap;

pub type SelectedPlan<VM> = MyGenCopy<VM>;

pub const ALLOC_MGC: AllocationSemantics = AllocationSemantics::Default;
pub const NURSERY_SIZE: usize = 16 * 1024 * 1024;

pub struct MyGenCopy<VM: VMBinding> {
    pub nursery: CopySpace<VM>,
    pub mature: ImmortalSpace<VM>,
    pub common: CommonPlan<VM>,
    in_nursery: AtomicBool,
    pub scheduler: &'static MMTkScheduler<VM>,
}

unsafe impl<VM: VMBinding> Sync for MyGenCopy<VM> {}

impl<VM: VMBinding> Plan for MyGenCopy<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = MGCCopyContext<VM>;

    fn collection_required(&self, space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
    Self: Sized,
    {
        let nursery_full = self.nursery.reserved_pages() >= (NURSERY_SIZE >> LOG_BYTES_IN_PAGE);
        let heap_full = self.get_pages_reserved() > self.get_total_pages();
        space_full || nursery_full || heap_full
    }

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END); //not sure if this should be mut

        MyGenCopy {
            nursery: CopySpace::new(
                "Nursery",
                false,
                true,
                VMRequest::fixed_extent(NURSERY_SIZE, false),
                vm_map,
                mmapper,
                &mut heap,
            ),
            mature: ImmortalSpace::new(
                "Mature",
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
            in_nursery: AtomicBool::default(),
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
        self.mature.init(&vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        
        
        

        let in_nursery = !self.request_full_heap_collection();
        self.in_nursery.store(in_nursery, Ordering::SeqCst);


        println!("MyGenCopy!! {}", in_nursery);
        // static mut A: usize = 0;
        // unsafe {
        //     debug_assert!(A == 0);
        //     A += 1;
        // }
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop & scan mutators
        if in_nursery {
            scheduler
                .unconstrained_works
                .add(StopMutators::<MGCNurseryProcessEdges<VM>>::new());
        } else {
            scheduler
                .unconstrained_works
                .add(StopMutators::<MGCMatureProcessEdges<VM>>::new());
        }
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        debug_assert!(cfg!(feature = "sanity"));
        #[cfg(feature = "sanity")]
        scheduler.final_stage.add(ScheduleSanityGC);
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn bind_mutator(
        //again, copy of semispace
        &'static self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_mgc_mutator(tls, mmtk))
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, tls: OpaquePointer) {
        //no flipping necessary
        
        self.common.prepare(tls, true);
        self.nursery.prepare(true);
    }

    fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);

        //release collected nursery
        self.nursery.release();
    }

    fn get_collection_reserve(&self) -> usize {
        //not sure what this function is supposed to do. Mayb need to change
        self.nursery.reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.nursery.reserved_pages() + self.mature.reserved_pages() + self.common.get_pages_used()
    }

    fn base (&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn in_nursery(&self) -> bool {
        self.in_nursery.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> MyGenCopy<VM> {
    fn request_full_heap_collection(&self) -> bool {
        self.get_total_pages() <= self.get_pages_reserved()
    }
}