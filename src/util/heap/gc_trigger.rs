use atomic::Ordering;

use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::options::{GCTriggerSelector, Options};
use crate::vm::VMBinding;
use crate::MMTK;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicUsize;

/// GCTrigger is responsible for triggering GCs based on the given policy.
/// All the decisions about heap limit and GC triggering should be resolved here.
/// Depending on the actual policy, we may either forward the calls either to the plan
/// or to the binding/runtime.
pub struct GCTrigger<VM: VMBinding> {
    /// The current plan. This is uninitialized when we create it, and later initialized
    /// once we have a fixed address for the plan.
    plan: MaybeUninit<&'static dyn Plan<VM = VM>>,
    /// The triggering policy.
    pub policy: Box<dyn GCTriggerPolicy<VM>>,
}

impl<VM: VMBinding> GCTrigger<VM> {
    pub fn new(options: &Options) -> Self {
        GCTrigger {
            plan: MaybeUninit::uninit(),
            policy: match *options.gc_trigger {
                GCTriggerSelector::FixedHeapSize(size) => Box::new(FixedHeapSizeTrigger {
                    total_pages: size >> LOG_BYTES_IN_PAGE,
                }),
                GCTriggerSelector::DynamicHeapSize(min, max) => Box::new(MemBalancerTrigger::new(
                    min >> LOG_BYTES_IN_PAGE,
                    max >> LOG_BYTES_IN_PAGE,
                )),
                GCTriggerSelector::Delegated => unimplemented!(),
            },
        }
    }

    /// Set the plan. This is called in `create_plan()` after we created a boxed plan.
    pub fn set_plan(&mut self, plan: &'static dyn Plan<VM = VM>) {
        self.plan.write(plan);
    }

    /// This method is called periodically by the allocation subsystem
    /// (by default, each time a page is consumed), and provides the
    /// collector with an opportunity to collect.
    ///
    /// Arguments:
    /// * `space_full`: Space request failed, must recover pages within 'space'.
    /// * `space`: The space that triggered the poll. This could `None` if the poll is not triggered by a space.
    pub fn poll(&self, space_full: bool, space: Option<&dyn Space<VM>>) -> bool {
        let plan = unsafe { self.plan.assume_init() };
        if self.policy.is_gc_required(space_full, space, plan) {
            info!(
                "[POLL] {}{}",
                if let Some(space) = space {
                    format!("{}: ", space.get_name())
                } else {
                    "".to_string()
                },
                "Triggering collection"
            );
            plan.base().gc_requester.request();
            return true;
        }
        false
    }

    /// Check if the heap is full
    pub fn is_heap_full(&self) -> bool {
        let plan = unsafe { self.plan.assume_init() };
        self.policy.is_heap_full(plan)
    }
}

/// This trait describes a GC trigger policy. A triggering policy have hooks to be informed about
/// GC start/end so they can collect some statistics about GC and allocation. The policy needs to
/// decide the (current) heap limit and decide whether a GC should be performed.
pub trait GCTriggerPolicy<VM: VMBinding>: Sync + Send {
    /// Inform the triggering policy that a GC starts.
    fn on_gc_start(&self, _mmtk: &'static MMTK<VM>) {}
    /// Inform the triggering policy that a GC ends.
    fn on_gc_end(&self, _mmtk: &'static MMTK<VM>) {}
    /// Is a GC required now?
    fn is_gc_required(
        &self,
        space_full: bool,
        space: Option<&dyn Space<VM>>,
        plan: &dyn Plan<VM = VM>,
    ) -> bool;
    /// Is current heap full?
    fn is_heap_full(&self, plan: &'static dyn Plan<VM = VM>) -> bool;
    /// Return the current heap size (in pages)
    fn get_heap_size_in_pages(&self) -> usize;
    /// Can the heap size grow?
    fn can_heap_size_grow(&self) -> bool;
}

/// A simple GC trigger that uses a fixed heap size.
pub struct FixedHeapSizeTrigger {
    total_pages: usize,
}
impl<VM: VMBinding> GCTriggerPolicy<VM> for FixedHeapSizeTrigger {
    fn is_gc_required(
        &self,
        space_full: bool,
        space: Option<&dyn Space<VM>>,
        plan: &dyn Plan<VM = VM>,
    ) -> bool {
        // Let the plan decide
        plan.collection_required(space_full, space)
    }

    fn is_heap_full(&self, plan: &'static dyn Plan<VM = VM>) -> bool {
        // If reserved pages is larger than the total pages, the heap is full.
        plan.get_reserved_pages() > self.total_pages
    }

    fn get_heap_size_in_pages(&self) -> usize {
        self.total_pages
    }

    fn can_heap_size_grow(&self) -> bool {
        false
    }
}

/// An implementation of MemBalancer (Optimal heap limits for reducing browser memory use, <https://dl.acm.org/doi/10.1145/3563323>)
/// We use MemBalancer to decide a heap limit between the min heap and the max heap.
/// The current implementation is a simplified version of mem balancer and it does not take collection/allocation speed into account,
/// and uses a fixed constant instead.
// TODO: implement a complete mem balancer.
pub struct MemBalancerTrigger {
    /// The min heap size
    min_heap_pages: usize,
    /// The max heap size
    max_heap_pages: usize,
    /// The current heap size
    current_heap_pages: AtomicUsize,
}
impl<VM: VMBinding> GCTriggerPolicy<VM> for MemBalancerTrigger {
    fn on_gc_end(&self, mmtk: &'static MMTK<VM>) {
        // live memory after a GC
        // Use reserved pages here: reserved pages includes the pending allocation requests that haven't been completed. Using
        // reserved pages makes sure that the new heap size could accomodate those pending allocation.
        // Otherwise, we may get into a stuck state where our computed heap size does not accomodate the next allocation,
        // and a GC is triggered. But the GC cannot collect anything, thus live bytes does not change, and the heap size
        // does not update. And we still cannot accomodate the next allocation. We have to avoid this, and make sure
        // our computed heap size works for the currently pending allocation.
        let live = mmtk.plan.get_reserved_pages() as f64;
        // We use a simplified version of mem balancer. Instead of collecting allocation/collection speed and a constant c,
        // we use a fixed constant 4096 instead.
        let optimal_heap = (live + (live * 4096f64).sqrt()) as usize;
        // The new heap size must be within min/max.
        let new_heap = optimal_heap.clamp(self.min_heap_pages, self.max_heap_pages);
        debug!(
            "MemBalander: new heap limit = {} pages (optimal = {}, clamped to [{}, {}])",
            new_heap, optimal_heap, self.min_heap_pages, self.max_heap_pages
        );
        self.current_heap_pages.store(new_heap, Ordering::Relaxed);
    }

    fn is_gc_required(
        &self,
        space_full: bool,
        space: Option<&dyn Space<VM>>,
        plan: &dyn Plan<VM = VM>,
    ) -> bool {
        // Let the plan decide
        plan.collection_required(space_full, space)
    }

    fn is_heap_full(&self, plan: &'static dyn Plan<VM = VM>) -> bool {
        // If reserved pages is larger than the current heap size, the heap is full.
        plan.get_reserved_pages() > self.current_heap_pages.load(Ordering::Relaxed)
    }

    fn get_heap_size_in_pages(&self) -> usize {
        self.current_heap_pages.load(Ordering::Relaxed)
    }

    fn can_heap_size_grow(&self) -> bool {
        self.current_heap_pages.load(Ordering::Relaxed) < self.max_heap_pages
    }
}
impl MemBalancerTrigger {
    fn new(min_heap_pages: usize, max_heap_pages: usize) -> Self {
        Self {
            min_heap_pages,
            max_heap_pages,
            // start with min heap
            current_heap_pages: AtomicUsize::new(min_heap_pages),
        }
    }
}
