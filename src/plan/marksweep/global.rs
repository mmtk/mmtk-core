use super::gc_works::{MSProcessEdges};
use crate::{mmtk::MMTK, plan::{self, global::NoCopy}, policy::space::Space, util::{Address, ObjectReference}};
use crate::policy::malloc::*;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::marksweep::mutator::create_ms_mutator;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::sync::Arc;

use atomic::Ordering;
use enum_map::EnumMap;

pub type SelectedPlan<VM> = MarkSweep<VM>;

//pub const ALLOC_MS: AllocationSemantics = AllocationSemantics::Default;

pub struct MarkSweep<VM: VMBinding> {
    pub common: CommonPlan<VM>,
    //pub mark_count: Mutex<u8>
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
            //mark_count: Mutex::new(1)
        }
    }

    fn is_malloced(&self, object: ObjectReference) -> bool {
        NODES.lock().unwrap().contains(&object)
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
        //println!("MarkSweep!!");
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
        println!("called marksweep release");
        unsafe {
            let mut NODES_hs = &mut *NODES.lock().unwrap();
            NODES_hs.retain(|&o| MarkSweep::<VM>::marked(&o));
            for object in NODES_hs.iter() {
                let a: Address = Address::from_usize(object.to_address().as_usize() - 8);
                let marking_word: usize = unsafe { a.load() };
                debug_assert!(marking_word != 0usize, "Marking word is 0, should have been removed from NODES");
                debug_assert!(marking_word == 1usize, "Marking word must be 1 or 0, found {}", marking_word);
                a.store(0);
            }
        }
    }

    fn get_collection_reserve(&self) -> usize {
        unreachable!("get col res");
        //self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        let mem = MEMORY_ALLOCATED.lock().unwrap();
        MALLOC_MEMORY - *mem
        // unreachable!("get pag use");
        //self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    // fn handle_user_collection_request(&self, tls: OpaquePointer, force: bool) {
    //     if force || !self.options().ignore_system_g_c {
    //         self.base()
    //             .user_triggered_collection
    //             .store(true, Ordering::Relaxed);
    //         self.base().control_collector_context.request();
    //         <Self::VM as VMBinding>::VMCollection::stop_all_mutators(tls);
    //     }
    // }
}

impl<VM: VMBinding> MarkSweep<VM> {
    fn marked(&object: &ObjectReference) -> bool {
        unsafe {
            let a: Address = Address::from_usize(object.to_address().as_usize() - 8);
            let marking_word: usize = unsafe { a.load() };
            let mut mem = MEMORY_ALLOCATED.lock().unwrap();
            if marking_word == 0 {
                println!("allocated: {}", *mem);
                let obj_size = libc::malloc_usable_size(a.to_mut_ptr());
                // debug_assert!(*MEMORY_MAP.lock().unwrap().get(&a.to_object_reference()).unwrap() == obj_size, "object was stored with size {} but released with size {}",MEMORY_MAP.lock().unwrap().get(&a.to_object_reference()).unwrap(),obj_size);
                debug_assert!(*mem >= obj_size, "Attempting to free an object sized {} when total memory allocated is {}", obj_size, mem);
                *mem -= obj_size;
                //println!("freeing object sized {}, memory allocated now {}", obj_size, mem);
                debug_assert!(*mem >= 0, "amount of memory allocated cannot be negative!");
                libc::free(a.to_mut_ptr()); 
                return false
            }
            true
        }
    }
}
