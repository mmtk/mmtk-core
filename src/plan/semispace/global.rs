use crate::policy::space::Space;

use super::SSMutator;
use crate::plan::trace::Trace;
use crate::plan::Allocator;
use crate::plan::Phase;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::util::heap::VMRequest;
use crate::util::OpaquePointer;
use std::cell::UnsafeCell;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::scheduler::*;
use crate::scheduler::gc_works::*;
use crate::mmtk::MMTK;
use super::gc_works::{SSCopyContext, SSProcessEdges};



pub type SelectedPlan<VM> = SemiSpace<VM>;

pub const ALLOC_SS: Allocator = Allocator::Default;

pub struct SemiSpace<VM: VMBinding> {
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
}

unsafe impl<VM: VMBinding> Sync for SemiSpace<VM> {}

impl<VM: VMBinding> Plan for SemiSpace<VM> {
    type VM = VM;
    type Mutator = SSMutator<VM>;
    type CopyContext = SSCopyContext<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        SemiSpace {
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
        }
    }

    fn gc_init(&mut self, heap_size: usize, mmtk: &'static MMTK<VM>) {
        self.common.gc_init(heap_size, mmtk);

        self.copyspace0.init(&mmtk.vm_map);
        self.copyspace1.init(&mmtk.vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.unconstrained_works.add(Initiate::<Self>::new());
        // Create initial works for `closure_stage`
        scheduler.unconstrained_works.add(StopMutators::<SSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        // Resume mutators
        scheduler.final_stage.add(ResumeMutators);
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<SSMutator<VM>> {
        Box::new(SSMutator::new(tls, self))
    }

    fn prepare(&self, tls: OpaquePointer) {
        self.common.prepare(tls, true);

        #[cfg(feature = "sanity")] self.fromspace().unprotect();

        self.hi.store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
        // prepare each of the collected regions
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);

        #[cfg(feature = "sanity")] {
            use crate::util::sanity::sanity_checker::SanityChecker;
            println!("Pre GC sanity check");
            SanityChecker::new(tls, &self).check();
        }

    }

    fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);
        // #[cfg(feature = "sanity")]
        // {
        //     use crate::util::constants::LOG_BYTES_IN_PAGE;
        //     use libc::memset;
        //     if self.fromspace().common().contiguous {
        //         let fromspace_start = self.fromspace().common().start;
        //         let fromspace_commited =
        //             self.fromspace().get_page_resource().committed_pages();
        //         let commited_bytes = fromspace_commited * (1 << LOG_BYTES_IN_PAGE);
        //         println!(
        //             "Destroying fromspace {}~{}",
        //             fromspace_start,
        //             fromspace_start + commited_bytes
        //         );
        //         memset(fromspace_start.to_mut_ptr(), 0xFF, commited_bytes);
        //     } else {
        //         println!("Fromspace is discontiguous, not destroying")
        //     }
        // }
        // release the collected region
        self.fromspace().release();
    }

    fn get_collection_reserve(&self) -> usize {
        self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> SemiSpace<VM> {
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
}
