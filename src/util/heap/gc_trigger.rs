use atomic::Ordering;

use crate::plan::gc_requester::GCRequester;
use crate::util::options::Options;
use crate::vm::VMBinding;
use crate::MMTK;
use crate::policy::space::Space;
use crate::plan::Plan;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

pub trait GCTrigger<VM: VMBinding>: Sync + Send {
    fn on_gc_start(&self, mmtk: &'static MMTK<VM>);
    fn on_gc_end(&self, mmtk: &'static MMTK<VM>);
    fn is_gc_required(&self, space_full: bool, space: Option<&dyn Space<VM>>, plan: &dyn Plan<VM=VM>) -> bool;
    fn is_heap_full(&self, plan: &'static dyn Plan<VM=VM>) -> bool;
    fn get_heap_size_in_pages(&self) -> usize;
    fn poll(&self, space_full: bool, space: Option<&dyn Space<VM>>, plan: &dyn Plan<VM = VM>) -> bool {
        if self.is_gc_required(space_full, space, plan) {
            plan.base().gc_requester.request();
            return true;
        }
        false
    }
}

pub fn create_gc_trigger<VM: VMBinding>(options: &Options) -> Arc<dyn GCTrigger<VM>> {
    todo!()
}

pub struct FixedHeapSizeTrigger {
    total_pages: usize,
}
impl<VM: VMBinding> GCTrigger<VM> for FixedHeapSizeTrigger {
    fn on_gc_start(&self, mmtk: &'static MMTK<VM>) {

    }

    fn on_gc_end(&self, mmtk: &'static MMTK<VM>) {

    }

    fn is_gc_required(&self, space_full: bool, space: Option<&dyn Space<VM>>, plan: &dyn Plan<VM=VM>) -> bool {
        plan.collection_required(space_full, space)
    }

    fn is_heap_full(&self, plan: &'static dyn Plan<VM=VM>) -> bool {
        plan.get_reserved_pages() > self.total_pages
    }

    fn get_heap_size_in_pages(&self) -> usize {
        self.total_pages
    }
}

pub struct MemBalancerTrigger {
    min_heap_pages: usize,
    max_heap_pages: usize,
    current_heap_pages: AtomicUsize,
}
impl<VM: VMBinding> GCTrigger<VM> for MemBalancerTrigger {
    fn on_gc_start(&self, mmtk: &'static MMTK<VM>) {

    }

    fn on_gc_end(&self, mmtk: &'static MMTK<VM>) {
        let live = mmtk.plan.get_used_pages() as f64;
        let new_heap = (live + (live * 4096f64).sqrt()) as usize;
        self.current_heap_pages.store(new_heap.clamp(self.min_heap_pages, self.max_heap_pages), Ordering::Relaxed);
    }

    fn is_gc_required(&self, space_full: bool, space: Option<&dyn Space<VM>>, plan: &dyn Plan<VM=VM>) -> bool {
        plan.collection_required(space_full, space)
    }

    fn is_heap_full(&self, plan: &'static dyn Plan<VM=VM>) -> bool {
        plan.get_reserved_pages() > self.current_heap_pages.load(Ordering::Relaxed)
    }

    fn get_heap_size_in_pages(&self) -> usize {
        self.current_heap_pages.load(Ordering::Relaxed)
    }
}
