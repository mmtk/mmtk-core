use atomic::Ordering;

use crate::plan::gc_requester::GCRequester;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::options::{Options, GCTriggerPolicySelector};
use crate::vm::VMBinding;
use crate::MMTK;
use crate::policy::space::Space;
use crate::plan::Plan;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::mem::MaybeUninit;

pub struct GCTrigger<VM: VMBinding> {
    plan: MaybeUninit<&'static dyn Plan<VM = VM>>,
    policy: Box<dyn GCTriggerPolicy<VM>>,
}

impl<VM: VMBinding> GCTrigger<VM> {
    pub fn new(options: &Options) -> Self {
        GCTrigger {
            plan: MaybeUninit::uninit(),
            policy: match *options.gc_trigger {
                GCTriggerPolicySelector::FixedHeapSize(size) => Box::new(FixedHeapSizeTrigger{
                    total_pages: size >> LOG_BYTES_IN_PAGE,
                }),
                GCTriggerPolicySelector::DynamicHeapSize(min, max) => Box::new(MemBalancerTrigger {
                    min_heap_pages: min >> LOG_BYTES_IN_PAGE,
                    max_heap_pages: max >> LOG_BYTES_IN_PAGE,
                    current_heap_pages: AtomicUsize::new(min >> LOG_BYTES_IN_PAGE)
                }),
                GCTriggerPolicySelector::Delegated => unimplemented!()
            }
        }
    }

    pub fn set_plan(&mut self, plan: &'static dyn Plan<VM = VM>) {
        self.plan.write(plan);
    }

    pub fn poll(&self, space_full: bool, space: Option<&dyn Space<VM>>) -> bool {
        let plan = unsafe { self.plan.assume_init() };
        if self.policy.is_gc_required(space_full, space, plan) {
            plan.base().gc_requester.request();
            return true;
        }
        false
    }

    pub fn get_total_pages(&self) -> usize {
        self.policy.get_heap_size_in_pages()
    }

    pub fn is_heap_full(&self) -> bool {
        let plan = unsafe { self.plan.assume_init() };
        self.policy.is_heap_full(plan)
    }
}

pub trait GCTriggerPolicy<VM: VMBinding>: Sync + Send {
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

pub fn create_gc_trigger<VM: VMBinding>(options: &Options) -> Arc<dyn GCTriggerPolicy<VM>> {
    match *options.gc_trigger {
        GCTriggerPolicySelector::FixedHeapSize(size) => Arc::new(FixedHeapSizeTrigger{
            total_pages: size >> LOG_BYTES_IN_PAGE,
        }),
        GCTriggerPolicySelector::DynamicHeapSize(min, max) => Arc::new(MemBalancerTrigger {
            min_heap_pages: min >> LOG_BYTES_IN_PAGE,
            max_heap_pages: max >> LOG_BYTES_IN_PAGE,
            current_heap_pages: AtomicUsize::new(min >> LOG_BYTES_IN_PAGE)
        }),
        GCTriggerPolicySelector::Delegated => unimplemented!()
    }
}

pub struct FixedHeapSizeTrigger {
    total_pages: usize,
}
impl<VM: VMBinding> GCTriggerPolicy<VM> for FixedHeapSizeTrigger {
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
impl<VM: VMBinding> GCTriggerPolicy<VM> for MemBalancerTrigger {
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
