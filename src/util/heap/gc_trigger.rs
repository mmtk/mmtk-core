use atomic::Ordering;

use crate::global_state::GlobalState;
use crate::plan::gc_requester::GCRequester;
use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::conversions;
use crate::util::options::{GCTriggerSelector, Options};
use crate::vm::VMBinding;
use crate::MMTK;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

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
    gc_requester: Arc<GCRequester<VM>>,
    options: Arc<Options>,
    state: Arc<GlobalState>,
}

impl<VM: VMBinding> GCTrigger<VM> {
    pub fn new(
        options: Arc<Options>,
        gc_requester: Arc<GCRequester<VM>>,
        state: Arc<GlobalState>,
    ) -> Self {
        GCTrigger {
            plan: MaybeUninit::uninit(),
            policy: match *options.gc_trigger {
                GCTriggerSelector::FixedHeapSize(size) => Box::new(FixedHeapSizeTrigger {
                    total_pages: conversions::bytes_to_pages_up(size),
                }),
                GCTriggerSelector::DynamicHeapSize(min, max) => Box::new(MemBalancerTrigger::new(
                    conversions::bytes_to_pages_up(min),
                    conversions::bytes_to_pages_up(max),
                )),
                GCTriggerSelector::Delegated => unimplemented!(),
            },
            options,
            gc_requester,
            state,
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
                "[POLL] {}{} ({}/{} pages)",
                if let Some(space) = space {
                    format!("{}: ", space.get_name())
                } else {
                    "".to_string()
                },
                "Triggering collection",
                plan.get_reserved_pages(),
                plan.get_total_pages(),
            );
            self.gc_requester.request();
            return true;
        }
        false
    }

    pub fn should_do_stress_gc(&self) -> bool {
        Self::should_do_stress_gc_inner(&self.state, &self.options)
    }

    /// Check if we should do a stress GC now. If GC is initialized and the allocation bytes exceeds
    /// the stress factor, we should do a stress GC.
    pub(crate) fn should_do_stress_gc_inner(state: &GlobalState, options: &Options) -> bool {
        state.is_initialized()
            && (state.allocation_bytes.load(Ordering::SeqCst) > *options.stress_factor)
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
    /// Inform the triggering policy that we have pending allocation.
    /// Any GC trigger policy with dynamic heap size should take this into account when calculating a new heap size.
    /// Failing to do so may result in unnecessay GCs, or result in an infinite loop if the new heap size
    /// can never accomodate the pending allocation.
    fn on_pending_allocation(&self, _pages: usize) {}
    /// Inform the triggering policy that a GC starts.
    fn on_gc_start(&self, _mmtk: &'static MMTK<VM>) {}
    /// Inform the triggering policy that a GC is about to start the release work. This is called
    /// in the global [`crate::scheduler::gc_work::Release`] work packet. This means we assume a plan
    /// do not schedule any work that reclaims memory before the global `Release` work. The current plans
    /// satisfy this assumption: they schedule other release work in `plan.release()`.
    fn on_gc_release(&self, _mmtk: &'static MMTK<VM>) {}
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
    fn is_heap_full(&self, plan: &dyn Plan<VM = VM>) -> bool;
    /// Return the current heap size (in pages)
    fn get_current_heap_size_in_pages(&self) -> usize;
    /// Return the upper bound of heap size
    fn get_max_heap_size_in_pages(&self) -> usize;
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

    fn is_heap_full(&self, plan: &dyn Plan<VM = VM>) -> bool {
        // If reserved pages is larger than the total pages, the heap is full.
        plan.get_reserved_pages() > self.total_pages
    }

    fn get_current_heap_size_in_pages(&self) -> usize {
        self.total_pages
    }

    fn get_max_heap_size_in_pages(&self) -> usize {
        self.total_pages
    }

    fn can_heap_size_grow(&self) -> bool {
        false
    }
}

use atomic_refcell::AtomicRefCell;
use std::time::Instant;

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
    /// The number of pending allocation pages. The allocation requests for them have failed, and a GC is triggered.
    /// We will need to take them into consideration so that the new heap size can accomodate those allocations.
    pending_pages: AtomicUsize,
    /// Statistics
    stats: AtomicRefCell<MemBalancerStats>,
}

#[derive(Copy, Clone, Debug)]
struct MemBalancerStats {
    // Allocation/collection stats in the previous estimation. We keep this so we can use them to smooth the current value
    /// Previous allocated memory in pages.
    allocation_pages_prev: Option<f64>,
    /// Previous allocation duration in secs
    allocation_time_prev: Option<f64>,
    /// Previous collected memory in pages
    collection_pages_prev: Option<f64>,
    /// Previous colleciton duration in secs
    collection_time_prev: Option<f64>,

    // Allocation/collection stats in this estimation.
    /// Allocated memory in pages
    allocation_pages: f64,
    /// Allocation duration in secs
    allocation_time: f64,
    /// Collected memory in pages (memory traversed during collection)
    collection_pages: f64,
    /// Collection duration in secs
    collection_time: f64,

    /// The time when this GC starts
    gc_start_time: Instant,
    /// The time when this GC ends
    gc_end_time: Instant,

    /// The live pages before we release memory.
    gc_release_live_pages: usize,
    /// The live pages at the GC end
    gc_end_live_pages: usize,
}

impl std::default::Default for MemBalancerStats {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            allocation_pages_prev: None,
            allocation_time_prev: None,
            collection_pages_prev: None,
            collection_time_prev: None,
            allocation_pages: 0f64,
            allocation_time: 0f64,
            collection_pages: 0f64,
            collection_time: 0f64,
            gc_start_time: now,
            gc_end_time: now,
            gc_release_live_pages: 0,
            gc_end_live_pages: 0,
        }
    }
}

use crate::plan::GenerationalPlan;

impl MemBalancerStats {
    // Collect mem stats for generational plans:
    // * We ignore nursery GCs.
    // * allocation = objects in mature space = promoted + pretentured = live pages in mature space before release - live pages at the end of last mature GC
    // * collection = live pages in mature space at the end of GC -  live pages in mature space before release

    fn generational_mem_stats_on_gc_start<VM: VMBinding>(
        &mut self,
        _plan: &dyn GenerationalPlan<VM = VM>,
    ) {
        // We don't need to do anything
    }
    fn generational_mem_stats_on_gc_release<VM: VMBinding>(
        &mut self,
        plan: &dyn GenerationalPlan<VM = VM>,
    ) {
        if !plan.is_current_gc_nursery() {
            self.gc_release_live_pages = plan.get_mature_reserved_pages();

            // Calculate the promoted pages (including pre tentured objects)
            let promoted = self
                .gc_release_live_pages
                .saturating_sub(self.gc_end_live_pages);
            self.allocation_pages = promoted as f64;
            trace!(
                "promoted = mature live before release {} - mature live at prev gc end {} = {}",
                self.gc_release_live_pages,
                self.gc_end_live_pages,
                promoted
            );
            trace!(
                "allocated pages (accumulated to) = {}",
                self.allocation_pages
            );
        }
    }
    /// Return true if we should compute a new heap limit. Only do so at the end of a mature GC
    fn generational_mem_stats_on_gc_end<VM: VMBinding>(
        &mut self,
        plan: &dyn GenerationalPlan<VM = VM>,
    ) -> bool {
        if !plan.is_current_gc_nursery() {
            self.gc_end_live_pages = plan.get_mature_reserved_pages();
            // Use live pages as an estimate for pages traversed during GC
            self.collection_pages = self.gc_end_live_pages as f64;
            trace!(
                "collected pages = mature live at gc end {} - mature live at gc release {} = {}",
                self.gc_release_live_pages,
                self.gc_end_live_pages,
                self.collection_pages
            );
            true
        } else {
            false
        }
    }

    // Collect mem stats for non generational plans
    // * allocation = live pages at the start of GC - live pages at the end of last GC
    // * collection = live pages at the end of GC - live pages before release

    fn non_generational_mem_stats_on_gc_start<VM: VMBinding>(&mut self, mmtk: &'static MMTK<VM>) {
        self.allocation_pages = mmtk
            .get_plan()
            .get_reserved_pages()
            .saturating_sub(self.gc_end_live_pages) as f64;
        trace!(
            "allocated pages = used {} - live in last gc {} = {}",
            mmtk.get_plan().get_reserved_pages(),
            self.gc_end_live_pages,
            self.allocation_pages
        );
    }
    fn non_generational_mem_stats_on_gc_release<VM: VMBinding>(&mut self, mmtk: &'static MMTK<VM>) {
        self.gc_release_live_pages = mmtk.get_plan().get_reserved_pages();
        trace!("live before release = {}", self.gc_release_live_pages);
    }
    fn non_generational_mem_stats_on_gc_end<VM: VMBinding>(&mut self, mmtk: &'static MMTK<VM>) {
        self.gc_end_live_pages = mmtk.get_plan().get_reserved_pages();
        trace!("live pages = {}", self.gc_end_live_pages);
        // Use live pages as an estimate for pages traversed during GC
        self.collection_pages = self.gc_end_live_pages as f64;
        trace!(
            "collected pages = live at gc end {} - live at gc release {} = {}",
            self.gc_release_live_pages,
            self.gc_end_live_pages,
            self.collection_pages
        );
    }
}

impl<VM: VMBinding> GCTriggerPolicy<VM> for MemBalancerTrigger {
    fn is_gc_required(
        &self,
        space_full: bool,
        space: Option<&dyn Space<VM>>,
        plan: &dyn Plan<VM = VM>,
    ) -> bool {
        // Let the plan decide
        plan.collection_required(space_full, space)
    }

    fn on_pending_allocation(&self, pages: usize) {
        self.pending_pages.fetch_add(pages, Ordering::SeqCst);
    }

    fn on_gc_start(&self, mmtk: &'static MMTK<VM>) {
        trace!("=== on_gc_start ===");
        self.access_stats(|stats| {
            stats.gc_start_time = Instant::now();
            stats.allocation_time += (stats.gc_start_time - stats.gc_end_time).as_secs_f64();
            trace!(
                "gc_start = {:?}, allocation_time = {}",
                stats.gc_start_time,
                stats.allocation_time
            );

            if let Some(plan) = mmtk.get_plan().generational() {
                stats.generational_mem_stats_on_gc_start(plan);
            } else {
                stats.non_generational_mem_stats_on_gc_start(mmtk);
            }
        });
    }

    fn on_gc_release(&self, mmtk: &'static MMTK<VM>) {
        trace!("=== on_gc_release ===");
        self.access_stats(|stats| {
            if let Some(plan) = mmtk.get_plan().generational() {
                stats.generational_mem_stats_on_gc_release(plan);
            } else {
                stats.non_generational_mem_stats_on_gc_release(mmtk);
            }
        });
    }

    fn on_gc_end(&self, mmtk: &'static MMTK<VM>) {
        trace!("=== on_gc_end ===");
        self.access_stats(|stats| {
            stats.gc_end_time = Instant::now();
            stats.collection_time += (stats.gc_end_time - stats.gc_start_time).as_secs_f64();
            trace!(
                "gc_end = {:?}, collection_time = {}",
                stats.gc_end_time,
                stats.collection_time
            );

            if let Some(plan) = mmtk.get_plan().generational() {
                if stats.generational_mem_stats_on_gc_end(plan) {
                    self.compute_new_heap_limit(
                        mmtk.get_plan().get_reserved_pages(),
                        // We reserve an extra of min nursery. This ensures that we will not trigger
                        // a full heap GC in the next GC (if available pages is smaller than min nursery, we will force a full heap GC)
                        mmtk.get_plan().get_collection_reserved_pages()
                            + mmtk.options.get_min_nursery_pages(),
                        stats,
                    );
                }
            } else {
                stats.non_generational_mem_stats_on_gc_end(mmtk);
                self.compute_new_heap_limit(
                    mmtk.get_plan().get_reserved_pages(),
                    mmtk.get_plan().get_collection_reserved_pages(),
                    stats,
                );
            }
        });
        // Clear pending allocation pages at the end of GC, no matter we used it or not.
        self.pending_pages.store(0, Ordering::SeqCst);
    }

    fn is_heap_full(&self, plan: &dyn Plan<VM = VM>) -> bool {
        // If reserved pages is larger than the current heap size, the heap is full.
        plan.get_reserved_pages() > self.current_heap_pages.load(Ordering::Relaxed)
    }

    fn get_current_heap_size_in_pages(&self) -> usize {
        self.current_heap_pages.load(Ordering::Relaxed)
    }

    fn get_max_heap_size_in_pages(&self) -> usize {
        self.max_heap_pages
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
            pending_pages: AtomicUsize::new(0),
            // start with min heap
            current_heap_pages: AtomicUsize::new(min_heap_pages),
            stats: AtomicRefCell::new(Default::default()),
        }
    }

    fn access_stats<F>(&self, mut f: F)
    where
        F: FnMut(&mut MemBalancerStats),
    {
        let mut stats = self.stats.borrow_mut();
        f(&mut stats);
    }

    fn compute_new_heap_limit(
        &self,
        live: usize,
        extra_reserve: usize,
        stats: &mut MemBalancerStats,
    ) {
        trace!("compute new heap limit: {:?}", stats);

        // Constants from the original paper
        const ALLOCATION_SMOOTH_FACTOR: f64 = 0.95;
        const COLLECTION_SMOOTH_FACTOR: f64 = 0.5;
        const TUNING_FACTOR: f64 = 0.2;

        // Smooth memory/time for allocation/collection
        let smooth = |prev: Option<f64>, cur, factor| {
            prev.map(|p| p * factor + cur * (1.0f64 - factor))
                .unwrap_or(cur)
        };
        let alloc_mem = smooth(
            stats.allocation_pages_prev,
            stats.allocation_pages,
            ALLOCATION_SMOOTH_FACTOR,
        );
        let alloc_time = smooth(
            stats.allocation_time_prev,
            stats.allocation_time,
            ALLOCATION_SMOOTH_FACTOR,
        );
        let gc_mem = smooth(
            stats.collection_pages_prev,
            stats.collection_pages,
            COLLECTION_SMOOTH_FACTOR,
        );
        let gc_time = smooth(
            stats.collection_time_prev,
            stats.collection_time,
            COLLECTION_SMOOTH_FACTOR,
        );
        trace!(
            "after smoothing, alloc mem = {}, alloc_time = {}",
            alloc_mem,
            alloc_time
        );
        trace!(
            "after smoothing, gc mem    = {}, gc_time    = {}",
            gc_mem,
            gc_time
        );

        // We got the smoothed stats. Now save the current stats as previous stats
        stats.allocation_pages_prev = Some(stats.allocation_pages);
        stats.allocation_pages = 0f64;
        stats.allocation_time_prev = Some(stats.allocation_time);
        stats.allocation_time = 0f64;
        stats.collection_pages_prev = Some(stats.collection_pages);
        stats.collection_pages = 0f64;
        stats.collection_time_prev = Some(stats.collection_time);
        stats.collection_time = 0f64;

        // Calculate the square root
        let e: f64 = if alloc_mem != 0f64 && gc_mem != 0f64 && alloc_time != 0f64 && gc_time != 0f64
        {
            let mut e = live as f64;
            e *= alloc_mem / alloc_time;
            e /= TUNING_FACTOR;
            e /= gc_mem / gc_time;
            e.sqrt()
        } else {
            // If any collected stat is abnormal, we use the fallback heuristics.
            (live as f64 * 4096f64).sqrt()
        };

        // Get pending allocations
        let pending_pages = self.pending_pages.load(Ordering::SeqCst);

        // This is the optimal heap limit due to mem balancer. We will need to clamp the value to the defined min/max range.
        let optimal_heap = live + e as usize + extra_reserve + pending_pages;
        trace!(
            "optimal = live {} + sqrt(live) {} + extra {}",
            live,
            e,
            extra_reserve
        );

        // The new heap size must be within min/max.
        let new_heap = optimal_heap.clamp(self.min_heap_pages, self.max_heap_pages);
        debug!(
            "MemBalander: new heap limit = {} pages (optimal = {}, clamped to [{}, {}])",
            new_heap, optimal_heap, self.min_heap_pages, self.max_heap_pages
        );
        self.current_heap_pages.store(new_heap, Ordering::Relaxed);
    }
}
