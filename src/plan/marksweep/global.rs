use super::gc_works::MSProcessEdges;
use crate::{mmtk::MMTK, util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK};
use crate::policy::malloc::HEAP_SIZE;
use crate::policy::malloc::METADATA_TABLE;
use crate::policy::malloc::malloc_usable_size;
use crate::policy::malloc::free;
use crate::policy::malloc::bitmap_index_to_address;
use crate::policy::malloc::HEAP_USED;
use crate::policy::mallocspace::MallocSpace;
use crate::plan::global::NoCopy;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::marksweep::mutator::create_ms_mutator;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::OpaquePointer;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::vm::VMBinding;
use std::sync::Arc;

use atomic::Ordering;
use enum_map::EnumMap;

pub type SelectedPlan<VM> = MarkSweep<VM>;

pub struct MarkSweep<VM: VMBinding> {
    pub common: CommonPlan<VM>,
    pub space: MallocSpace<VM>,
}

unsafe impl<VM: VMBinding> Sync for MarkSweep<VM> {}

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = NoCopy<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        MarkSweep {
            common: CommonPlan::new(vm_map, mmapper, options, heap),
            space: MallocSpace::new(),
        }
    }



    fn collection_required(&self, _space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
            Self: Sized, {
        unsafe { HEAP_USED.load(Ordering::SeqCst) >= HEAP_SIZE }
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        unsafe { HEAP_SIZE = heap_size; }
        self.common.gc_init(heap_size, vm_map, scheduler);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop and scan mutators
        scheduler
            .unconstrained_works
            .add(StopMutators::<MSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.final_stage.add(ScheduleSanityGC);
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn bind_mutator(
        &'static self,
        tls: OpaquePointer,
        _mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_ms_mutator(tls, self))
    }
    
    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, tls: OpaquePointer) {
        // Do nothing  
    }

    fn release(&self, tls: OpaquePointer) {
        unsafe {
                //using bitmaps
                let chunks = METADATA_TABLE.read().unwrap().len();
                {let ref mut metadata_table = METADATA_TABLE.write().unwrap();
                let mut chunk_index = 0;
                let mut malloc_count = 0;
                let mut mark_count = 0;
                while chunk_index < chunks {
                    let mut row = &mut metadata_table[chunk_index];
                    let ref mut malloced = row.1;
                    let ref mut marked = row.2;
                    let mut bitmap_index = 0;
                    while bitmap_index < BYTES_IN_CHUNK/16 {
                        if malloced[bitmap_index] == 1 {
                            malloc_count += 1;
                            if marked[bitmap_index] == 0 {
                                let chunk_start = row.0;
                                let address = bitmap_index_to_address(bitmap_index, chunk_start);
                                let ptr = address.to_mut_ptr();
                                let freed_memory = malloc_usable_size(ptr);
                                HEAP_USED.fetch_sub(freed_memory, Ordering::SeqCst);
                                free(ptr);
                                malloced[bitmap_index] = 0;
                                marked[bitmap_index] = 0;
                            } else {
                                marked[bitmap_index] = 0;
                                let chunk_start = row.0;
                                let address = bitmap_index_to_address(bitmap_index, chunk_start);
                                mark_count += 1;
                            }
                        }
                        bitmap_index += 1;
                    }
                    chunk_index += 1;
                }
            }
        }
    }

    fn get_collection_reserve(&self) -> usize {
        unimplemented!();
    }

    fn get_pages_used(&self) -> usize {
        self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}