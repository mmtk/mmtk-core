use super::gc_works::MSProcessEdges;
use crate::{mmtk::MMTK, util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK};
use crate::policy::malloc::*;
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
use crate::util::{Address, ObjectReference, OpaquePointer};
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::vm::VMBinding;
use std::{ops::Sub, sync::{Arc, atomic::AtomicU8}};

use atomic::Ordering;
use enum_map::EnumMap;

pub type SelectedPlan<VM> = MarkSweep<VM>;

pub struct MarkSweep<VM: VMBinding> {
    pub common: CommonPlan<VM>,
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
        }
    }



    fn collection_required(&self, space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
            Self: Sized, {
        unsafe { malloc_memory_full() }
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
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
        unsafe { PHASE = Phase::Sweeping; }
        println!("global::release()");
        {
            let total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
            println!("total memory allocated = {}", *total_memory_allocated);
        }
        unsafe {
            if USE_HASHSET {
                //using hashset
                let mut NODES_mut = &mut *NODES.lock().unwrap();
                NODES_mut.retain(|&o| MarkSweep::<VM>::marked(&o));
                for object in NODES_mut.iter() {
                    let a: Address = object.to_address().sub(8);
                    let marking_word: usize = a.load();
                    debug_assert!(marking_word != 0usize, "Marking word is 0, should have been removed from NODES");
                    debug_assert!(marking_word == 1usize, "Marking word must be 1 or 0, found {}", marking_word);
                    a.store(0);
                }
            } else {
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
                                let mut total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
                                let freed_memory = libc::malloc_usable_size(ptr);
                                *total_memory_allocated -= freed_memory;
                                libc::free(ptr);
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
                println!("done freeing, found {} objects malloced of which {} were marked", malloc_count, mark_count);

                }
                
                let total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
                println!("total memory allocated = {}", *total_memory_allocated);
                PHASE = Phase::Allocation;

            }


        }
    }

    fn get_collection_reserve(&self) -> usize {
        unimplemented!();
    }

    fn get_pages_used(&self) -> usize {
        let total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
        MALLOC_MEMORY - *total_memory_allocated
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> MarkSweep<VM> {
    fn marked(&object: &ObjectReference) -> bool {
        unsafe {
            let address: Address = object.to_address().sub(8);
            let marking_word: usize = address.load();
            let mut total_memory_allocated = MEMORY_ALLOCATED.lock().unwrap();
            if marking_word == 0 {
                let freed_memory = libc::malloc_usable_size(address.to_mut_ptr());
                debug_assert!(*total_memory_allocated >= freed_memory, "Attempting to free an object sized {} when total memory allocated is {}", freed_memory, total_memory_allocated);
                *total_memory_allocated -= freed_memory;
                debug_assert!(*total_memory_allocated >= 0, "amount of memory allocated cannot be negative!");
                // println!("Freed {} bytes, total {} bytes.", freed_memory, total_memory_allocated);
                libc::free(address.to_mut_ptr()); 
                return false
            }
            true
        }
    }
}
