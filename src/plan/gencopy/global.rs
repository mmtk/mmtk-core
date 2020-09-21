use crate::policy::space::Space;

use super::GenCopyMutator;
use crate::plan::Allocator;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::util::heap::VMRequest;
use crate::util::OpaquePointer;
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
use super::gc_works::{GenCopyCopyContext, GenCopyNurseryProcessEdges, GenCopyMatureProcessEdges};



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
}

unsafe impl<VM: VMBinding> Sync for GenCopy<VM> {}

impl <VM: VMBinding> Plan for GenCopy<VM> {
    type VM = VM;
    type Mutator = GenCopyMutator<VM>;
    type CopyContext = GenCopyCopyContext<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
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
        }
    }

    fn gc_init(&mut self, heap_size: usize, mmtk: &'static MMTK<VM>) {
        self.common.gc_init(heap_size, mmtk);
        self.nursery.init(&mmtk.vm_map);
        self.copyspace0.init(&mmtk.vm_map);
        self.copyspace1.init(&mmtk.vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        let in_nursery = !self.request_full_heap_collection();
        self.in_nursery.store(in_nursery, Ordering::SeqCst);

        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.unconstrained_works.add(Initiate::<Self>::new());
        // Create initial works for `closure_stage`
        if in_nursery {
            scheduler.unconstrained_works.add(StopMutators::<GenCopyNurseryProcessEdges<VM>>::new());
        } else {
            scheduler.unconstrained_works.add(StopMutators::<GenCopyMatureProcessEdges<VM>>::new());
        }
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        // Resume mutators
        scheduler.final_stage.add(ResumeMutators);
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<GenCopyMutator<VM>> {
        Box::new(GenCopyMutator::new(tls, self))
    }

    fn prepare(&self, tls: OpaquePointer) {
        self.common.prepare(tls, true);
        self.nursery.prepare(true);
        if !self.in_nursery() {
            self.hi.store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
        }
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
    }

    fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);
        self.nursery.release();
        if !self.in_nursery() {
            self.fromspace().release();
        }
    }

    fn get_collection_reserve(&self) -> usize {
        self.nursery.reserved_pages() + self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.nursery.reserved_pages() + self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}


impl <VM: VMBinding> GenCopy<VM> {
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

    pub fn in_nursery(&self) -> bool {
        self.in_nursery.load(Ordering::SeqCst)
    }
}