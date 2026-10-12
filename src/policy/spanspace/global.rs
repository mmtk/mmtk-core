//! `SpanSpace`: the space itself (SFT, `Space`, tracing, sweeping and the plan-facing API).

use super::central::Central;
use super::classes::*;
use super::pages::{FreeResult, PageAllocator, BYTES_IN_PAGE_REGION};
#[cfg(feature = "vo_bit")]
use super::sizeclasses::MAX_SMALL_SIZE;
use super::span::{Span, SpanId, SpanTable};
use super::sweep::{sweep_state, try_acquire, SweepSpans, SweepState};
use crate::plan::tracing::{ObjectQueue, OptionObjectQueue};
use crate::policy::gc_work::{PolicyTraceObject, TraceKind};
use crate::policy::sft::{GCWorkerMutRef, SFT};
use crate::policy::space::{CommonSpace, PlanCreateSpaceArgs, Space};
use crate::scheduler::{GCWork, GCWorkScheduler, GCWorker, WorkBucketStage};
use crate::util::alloc::allocator::AllocationOptions;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::copy::CopySemantics;
use crate::util::heap::{FreeListPageResource, PageResource};
use crate::util::metadata::side_metadata::spec_defs::SPAN_PAGE_MAP;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::object_enum::ObjectEnumerator;
use crate::util::{Address, ObjectReference, VMThread};
use crate::vm::{ActivePlan, ObjectModel, VMBinding};
use crossbeam::utils::CachePadded;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// A hook called by [`crate::util::alloc::SpanAllocator`] each time it has refilled a span (Go:
/// `mallocgc` checks the GC trigger when `nextFree` reports a refill, `runtime/malloc.go`, and
/// `deductAssistCredit` accounts for assists). The plan uses it to poll its trigger
/// (`heap_live >= trigger`, design §5.10), and later for assist accounting (§5.11). The
/// arguments are the mutator thread and the bytes the refill charged to `heap_live`. The hook
/// must not block for GC: `GCTrigger::poll` only requests a collection.
pub type RefillHook = Box<dyn Fn(VMThread, usize) + Send + Sync>;

/// Called by an allocator whose assist credit (Go's `gcAssistBytes`) is negative, at the start
/// of its slow path, with the credit.  The hook may block the mutator (a mark assist) and should
/// update the credit.  Nothing is cached or locked when it is called, so a GC pause can happen
/// while it blocks.
pub type AssistHook = Box<dyn Fn(VMThread, &mut isize) + Send + Sync>;

/// The number of stripes of the `bytes_marked` counter.
const BYTES_MARKED_STRIPES: usize = 32;

/// The initial heap `sweepgen`.
const INITIAL_SWEEPGEN: u32 = 0;

/// The span heap: Go's size-segregated heap of spans (`mheap` + `mcentral` + `pageAlloc`), as an
/// MMTk space. See the module documentation of [`crate::policy::spanspace`].
pub struct SpanSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// The page allocator within the space's page regions.
    pages: PageAllocator,
    /// Span descriptors.
    spans: SpanTable,
    /// Per-span-class central span sets.
    central: Central,
    /// The heap sweep generation `h` (Go: `mheap_.sweepgen`).
    sweepgen: AtomicU32,
    /// Set from `prepare_initial_mark` to `release_after_mark`: mark bits are being set.
    marking: AtomicBool,
    /// Bytes of live heap in this space as the pacer sees it (Go: `gcController.heapLive`):
    /// charged a whole span's free bytes at refill, corrected at uncache, and reset to the bytes
    /// marked at the end of marking.
    heap_live: AtomicIsize,
    /// Bytes marked in this space in the current cycle, including allocate-black (Go:
    /// `work.bytesMarked`). Striped by worker to avoid contention.
    bytes_marked: Box<[CachePadded<AtomicUsize>]>,
    /// SpanSpace pages swept since the last `release_after_mark` (Go: `mheap_.pagesSwept`).
    pages_swept: AtomicUsize,
    /// Pages swept by the last `prepare_initial_mark` (sweep termination residue).
    last_sweep_residue_pages: AtomicUsize,
    refill_hook: OnceLock<RefillHook>,
    assist_hook: OnceLock<AssistHook>,
}

impl<VM: VMBinding> SFT for SpanSpace<VM> {
    fn name(&self) -> &'static str {
        self.common.name
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        let Some(span) = self.span_of(Self::object_start(object)) else {
            return false;
        };
        let index = span.obj_index(Self::object_start(object));
        let h = self.sweepgen.load(Ordering::Acquire);
        if self.marking.load(Ordering::Acquire) {
            return span.is_marked(index);
        }
        match sweep_state(span.sweepgen(), h) {
            // Marked in the cycle that just ended and not swept yet.
            SweepState::NeedsSweep | SweepState::CachedStale => span.is_marked(index),
            // Being swept: the marks are moving into the alloc bits.
            SweepState::BeingSwept => span.is_marked(index) || span.is_allocated(index),
            // Swept: liveness is the allocation state.
            SweepState::Swept | SweepState::Cached => span.is_allocated(index),
        }
    }

    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, _object: ObjectReference) -> bool {
        false
    }

    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        false
    }

    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        true
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _bytes: usize) {
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit(object);
        if self.should_allocate_as_live() {
            self.allocate_black(object);
        }
    }

    #[cfg(feature = "vo_bit")]
    fn is_mmtk_object(&self, addr: Address) -> Option<ObjectReference> {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr(addr)
    }

    #[cfg(feature = "vo_bit")]
    fn find_object_from_internal_pointer(
        &self,
        ptr: Address,
        max_search_bytes: usize,
    ) -> Option<ObjectReference> {
        let search_bytes = usize::min(MAX_SMALL_SIZE, max_search_bytes);
        crate::util::metadata::vo_bit::find_object_from_internal_pointer::<VM>(ptr, search_bytes)
    }

    fn sft_trace_object(
        &self,
        queue: &mut OptionObjectQueue,
        object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }

    fn debug_print_object_info(&self, object: ObjectReference) {
        match self.span_of(Self::object_start(object)) {
            Some(span) => {
                let index = span.obj_index(Self::object_start(object));
                println!(
                    "span = [{}, {}), class = {:?}, elemsize = {}, index = {}, sweepgen = {} (h = {}), marked = {}, allocated = {}",
                    span.start(),
                    span.end(),
                    span.spanclass(),
                    span.elemsize(),
                    index,
                    span.sweepgen(),
                    self.sweepgen.load(Ordering::Relaxed),
                    span.is_marked(index),
                    span.is_allocated(index)
                );
            }
            None => println!("no span"),
        }
        self.common.debug_print_object_global_info(object);
    }
}

impl<VM: VMBinding> Space<VM> for SpanSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }

    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }

    fn maybe_get_page_resource_mut(&mut self) -> Option<&mut dyn PageResource<VM>> {
        Some(&mut self.pr)
    }

    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        self.common().initialize_sft(self.as_sft(), sft_map)
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unreachable!("SpanSpace releases its page regions itself")
    }

    fn enumerate_objects(&self, enumerator: &mut dyn ObjectEnumerator) {
        for (_, span) in self.spans.iter_in_use() {
            enumerator.visit_address_range(span.start(), span.end());
        }
    }

    fn clear_side_log_bits(&self) {
        let log_bit = VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.extract_side_spec();
        for region in self.pages.regions() {
            log_bit.bzero_metadata(region, BYTES_IN_PAGE_REGION);
        }
    }

    fn set_side_log_bits(&self) {
        let log_bit = VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.extract_side_spec();
        for region in self.pages.regions() {
            log_bit.bset_metadata(region, BYTES_IN_PAGE_REGION);
        }
    }
}

impl<VM: VMBinding> PolicyTraceObject<VM> for SpanSpace<VM> {
    fn trace_object<Q: ObjectQueue, const KIND: TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }

    fn may_move_objects<const KIND: TraceKind>() -> bool {
        false
    }
}

impl<VM: VMBinding> SpanSpace<VM> {
    pub fn new(args: PlanCreateSpaceArgs<VM>) -> Self {
        let scheduler = args.scheduler.clone();
        let vm_map = args.vm_map;
        let is_discontiguous = args.vmrequest.is_discontiguous();
        let local_specs = vec![SPAN_PAGE_MAP];
        let common = CommonSpace::new(args.into_policy_args(false, false, local_specs));
        let pr = if is_discontiguous {
            FreeListPageResource::new_discontiguous(vm_map)
        } else {
            FreeListPageResource::new_contiguous(common.start, common.extent, vm_map)
        };
        let stripes = BYTES_MARKED_STRIPES;
        SpanSpace {
            common,
            pr,
            scheduler,
            pages: PageAllocator::new(),
            spans: SpanTable::new(),
            central: Central::new(),
            sweepgen: AtomicU32::new(INITIAL_SWEEPGEN),
            marking: AtomicBool::new(false),
            heap_live: AtomicIsize::new(0),
            bytes_marked: (0..stripes)
                .map(|_| CachePadded::new(AtomicUsize::new(0)))
                .collect(),
            pages_swept: AtomicUsize::new(0),
            last_sweep_residue_pages: AtomicUsize::new(0),
            refill_hook: OnceLock::new(),
            assist_hook: OnceLock::new(),
        }
    }

    /// SpanSpace needs no global side metadata. (Exists so that plans can treat spaces alike.)
    #[allow(clippy::ptr_arg)]
    pub fn extend_global_side_metadata_specs(_specs: &mut Vec<SideMetadataSpec>) {}

    // ---------------------------------------------------------------------------------------
    // Plan-facing API
    // ---------------------------------------------------------------------------------------

    /// The heap sweep generation `h`.
    pub fn sweepgen(&self) -> u32 {
        self.sweepgen.load(Ordering::Acquire)
    }

    /// Sweep termination, at the start of a mark phase (Go: `finishsweep_m` in the
    /// sweep-termination STW of `gcStart`). Call it in the pause that starts marking
    /// (`Prepare` of `InitialMark`, or of a `Full` pause), with mutators stopped.
    ///
    /// - Sweeps any spans still unswept from the previous cycle, on this thread. Normally the plan
    ///   has already run the sweep packets (or `sweep_one`) before the pause, so this residue is
    ///   empty. Its size is returned (in SpanSpace pages) and kept as a pause-time metric.
    /// - Checks that every in-use span is swept (`h`) or cached by an allocator (`h + 3`).
    /// - Starts a new marking epoch: resets `bytes_marked` and makes `is_live` read mark bits.
    ///
    /// The mark bits of every span are already clear: sweeping clears them (see
    /// [`super::span`]), so there is no Prepare-time clearing (design §5.4).
    pub fn prepare_initial_mark(&self) -> usize {
        let mut residue = 0;
        loop {
            let swept = self.sweep_one();
            if swept == 0 {
                break;
            }
            residue += swept;
        }
        self.last_sweep_residue_pages
            .store(residue, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        self.assert_all_swept();
        for c in self.bytes_marked.iter() {
            c.store(0, Ordering::Relaxed);
        }
        self.marking.store(true, Ordering::SeqCst);
        residue
    }

    /// The end of marking (Go: `gcMarkTermination` → `gcSweep`): `sweepgen += 2`. Call it in
    /// the release of the pause that ends marking (`FinalMark`, or a `Full` pause), with mutators
    /// stopped, **before** the mutators' allocators are released
    /// ([`crate::util::alloc::SpanAllocator::release`]), which sweep their stale cached spans.
    ///
    /// Every in-use span becomes unswept. `heap_live` is reset to the bytes marked in this space
    /// (Go: `gcController.resetLive(work.bytesMarked)`). The plan then schedules sweeping with
    /// [`Self::schedule_sweep`], or leaves it to allocators and [`Self::sweep_one`].
    pub fn release_after_mark(&self) {
        self.marking.store(false, Ordering::SeqCst);
        let h = self.sweepgen.load(Ordering::Relaxed);
        self.sweepgen.store(h.wrapping_add(2), Ordering::SeqCst);
        self.pages_swept.store(0, Ordering::Relaxed);
        self.heap_live
            .store(self.bytes_marked() as isize, Ordering::Relaxed);
    }

    /// Schedule sweep packets (Go's background sweeper) in `stage`:
    /// - `Release`: eager sweeping in the pause that ends marking;
    /// - `Concurrent`: background sweeping after the pause (the plan must handle the bucket
    ///   draining and interruption, design §5.5);
    /// - `Prepare`: finish a previous cycle's sweeping in parallel at the start of a pause.
    ///
    /// Allocators sweep lazily in any case: whatever the packets have not swept when an
    /// allocator needs a span is swept by the allocator.
    pub fn schedule_sweep(&'static self, stage: WorkBucketStage) {
        let n = self.scheduler.num_workers().max(1);
        let packets: Vec<Box<dyn GCWork<VM>>> = (0..n)
            .map(|i| {
                Box::new(SweepSpans {
                    space: self,
                    first_class: i * NUM_SPAN_CLASSES / n,
                }) as _
            })
            .collect();
        let bucket = &self.scheduler.work_buckets[stage];
        if bucket.is_enabled() {
            bucket.bulk_add(packets);
        } else {
            // For example `Concurrent` during a pause.  The scheduler enables it when the pause
            // ends, because it is not empty.
            for packet in packets {
                bucket.add_boxed_no_notify(packet);
            }
        }
    }

    /// Whether any span is waiting to be swept by a sweeper (spans in the central unswept sets).
    /// Stale spans cached by allocators are not counted: their allocators sweep them when they
    /// are released.
    pub fn has_unswept(&self) -> bool {
        self.central.count_unswept(self.sweepgen()) != 0
    }

    /// Number of spans waiting to be swept.
    pub fn count_unswept(&self) -> usize {
        self.central.count_unswept(self.sweepgen())
    }

    /// Sweep one unswept span without allocating (Go: `sweepone`). Returns the number of
    /// SpanSpace pages swept, or 0 if there was nothing to sweep. The plan can use this to
    /// finish sweeping before it triggers a cycle (design §5.5, sweep termination), and for
    /// proportional sweeping.
    pub fn sweep_one(&self) -> usize {
        self.sweep_one_from(0)
    }

    pub(super) fn sweep_one_from(&self, first_class: usize) -> usize {
        let h = self.sweepgen.load(Ordering::Acquire);
        for i in 0..NUM_SPAN_CLASSES {
            let spc = SpanClass(((first_class + i) % NUM_SPAN_CLASSES) as u8);
            while let Some(id) = self.central.pop_unswept(spc, h) {
                let span = self.spans.get(id);
                if try_acquire(&span.sweepgen, h) {
                    let npages = span.npages();
                    self.sweep_span(id, false);
                    return npages;
                }
                // Somebody else owns it; they put it on the right list.
            }
        }
        0
    }

    /// Bytes marked in this space in the current cycle (Go: `work.bytesMarked`), including
    /// objects allocated black.
    pub fn bytes_marked(&self) -> usize {
        self.bytes_marked
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .sum()
    }

    /// Return the bytes marked so far and reset the counter.
    pub fn take_bytes_marked(&self) -> usize {
        self.bytes_marked
            .iter()
            .map(|c| c.swap(0, Ordering::Relaxed))
            .sum()
    }

    /// Live heap bytes in this space as Go's pacer sees it (Go: `heapLive`). See the field doc.
    pub fn heap_live(&self) -> usize {
        self.heap_live.load(Ordering::Relaxed).max(0) as usize
    }

    /// Install the refill hook (see [`RefillHook`]). Can only be set once.
    pub fn set_refill_hook(&self, hook: RefillHook) {
        if self.refill_hook.set(hook).is_err() {
            panic!("SpanSpace refill hook already set");
        }
    }

    /// Install the assist hook (see [`AssistHook`]) unless one is installed already.
    pub fn set_assist_hook_once(&self, hook: impl FnOnce() -> AssistHook) {
        self.assist_hook.get_or_init(hook);
    }

    /// Whether objects are being allocated black, i.e. marking is in progress.
    pub(crate) fn is_marking(&self) -> bool {
        self.should_allocate_as_live()
    }

    pub(crate) fn call_assist_hook(&self, tls: VMThread, credit: &mut isize) {
        if let Some(hook) = self.assist_hook.get() {
            hook(tls, credit);
        }
    }

    /// SpanSpace pages swept since the end of the last mark phase.
    pub fn pages_swept(&self) -> usize {
        self.pages_swept.load(Ordering::Relaxed)
    }

    /// SpanSpace pages that `prepare_initial_mark` had to sweep in the pause, last time.
    pub fn last_sweep_residue_pages(&self) -> usize {
        self.last_sweep_residue_pages.load(Ordering::Relaxed)
    }

    /// SpanSpace pages in use by spans.
    pub fn pages_in_use(&self) -> usize {
        self.pages.used_pages()
    }

    // ---------------------------------------------------------------------------------------
    // Tracing
    // ---------------------------------------------------------------------------------------

    /// The start of an object's cell. Object references may point anywhere in the object (as in
    /// MockVM), and the cell is found from the object start.
    #[inline(always)]
    fn object_start(object: ObjectReference) -> Address {
        VM::VMObjectModel::ref_to_object_start(object)
    }

    /// The span that contains `addr`, via the page map (Go: `spanOf`). `None` if the page is not
    /// in an in-use span.
    #[inline(always)]
    pub fn span_of(&self, addr: Address) -> Option<&Span> {
        let id: SpanId = SPAN_PAGE_MAP.load_atomic::<u32>(addr, Ordering::Relaxed);
        if id == 0 {
            return None;
        }
        let span = self.spans.get(id);
        if span.is_in_use() {
            Some(span)
        } else {
            None
        }
    }

    /// Mark an object (design §5.4, Go: `findObject` + `greyobject`): page map → span →
    /// `div_magic` object index → atomic OR of the mark bit. If the object is newly marked, its
    /// size is added to `bytes_marked`, and it is enqueued for scanning unless its span is
    /// noscan.
    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        let addr = Self::object_start(object);
        let span = self
            .span_of(addr)
            .unwrap_or_else(|| panic!("SpanSpace: {object} is not in an in-use span"));
        let index = span.obj_index(addr);
        debug_assert!(index < span.nelems());
        if span.try_mark(index) {
            self.add_bytes_marked(span.elemsize());
            if !span.spanclass().noscan() {
                queue.enqueue(object);
            }
        }
        object
    }

    #[inline(always)]
    fn add_bytes_marked(&self, bytes: usize) {
        let stripe =
            crate::scheduler::current_worker_ordinal().unwrap_or(0) % self.bytes_marked.len();
        self.bytes_marked[stripe].fetch_add(bytes, Ordering::Relaxed);
    }

    /// Allocate-black (Go: `gcmarknewobject`, `runtime/mgcmark.go:1762`): mark a new object and
    /// count its size as marked. With the SATB barrier option, also leave the new object logged,
    /// so the barrier ignores it (it is not part of the snapshot).
    fn allocate_black(&self, object: ObjectReference) {
        let addr = Self::object_start(object);
        let span = self
            .span_of(addr)
            .expect("new object not in an in-use span");
        let index = span.obj_index(addr);
        if span.try_mark(index) {
            self.add_bytes_marked(span.elemsize());
        }
        if self.common.needs_log_bit {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                .bulk_mark_as_logged(span.object_address(index), span.elemsize());
        }
    }

    // ---------------------------------------------------------------------------------------
    // Spans and pages (Go: mheap)
    // ---------------------------------------------------------------------------------------

    /// Allocate a span of class `spc` from the page allocator, acquiring a new page region from MMTk if
    /// needed (Go: `mheap.alloc`). Returns `None` if MMTk could not provide memory (a GC may have
    /// happened, in which case the caller retries).
    fn alloc_span(
        &self,
        tls: VMThread,
        spc: SpanClass,
        alloc_options: AllocationOptions,
    ) -> Option<(SpanId, &Span)> {
        let npages = spc.npages();
        let (start, needzero) = loop {
            if let Some(r) = self.pages.alloc(npages) {
                break r;
            }
            let region = self.acquire(
                tls,
                BYTES_IN_PAGE_REGION >> LOG_BYTES_IN_PAGE,
                alloc_options,
            );
            if region.is_zero() {
                return None;
            }
            assert!(
                region.is_aligned_to(BYTES_IN_PAGE_REGION),
                "SpanSpace got an unaligned page region {region}"
            );
            if self.common.needs_log_bit {
                // The region's log bits may be stale from a previous user of the memory.
                VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                    .bulk_mark_as_logged(region, BYTES_IN_PAGE_REGION);
            }
            self.pages.add_region(region, self.common.zeroed);
        };
        let id = self.spans.alloc_id();
        let span = self.spans.get(id);
        span.init(start, spc, self.sweepgen.load(Ordering::Acquire), needzero);
        self.set_page_map(start, npages, id);
        Some((id, span))
    }

    /// Return a span's pages (Go: `mheap.freeSpan`). Hands an empty page region back to MMTk.
    fn free_span(&self, id: SpanId, span: &Span) {
        let start = span.start();
        let npages = span.npages();
        self.set_page_map(start, npages, 0);
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::bzero_vo_bit(start, span.bytes());
        span.state
            .store(super::span::SpanState::Dead as u8, Ordering::Release);
        self.spans.free_id(id);
        if let FreeResult::RegionEmpty(region) = self.pages.free(start, npages) {
            // Decision D30: unlike Go, return empty page regions to MMTk, so that other spaces
            // can use the memory and MMTk's heap accounting sees it as free.
            self.pr.release_pages(region);
        }
    }

    fn set_page_map(&self, start: Address, npages: usize, id: SpanId) {
        for p in 0..npages {
            SPAN_PAGE_MAP.store_atomic::<u32>(
                start + (p << LOG_BYTES_IN_SPAN_PAGE),
                id,
                Ordering::Release,
            );
        }
    }

    /// Find the id of a span from its descriptor (via the page map).
    fn span_id_of(&self, span: &Span) -> SpanId {
        SPAN_PAGE_MAP.load_atomic::<u32>(span.start(), Ordering::Relaxed)
    }

    // ---------------------------------------------------------------------------------------
    // Allocator support (Go: mcentral)
    // ---------------------------------------------------------------------------------------

    pub(crate) fn call_refill_hook(&self, tls: VMThread, bytes: usize) {
        // Go checks the GC trigger after every refill (`mcache.refill`, then `mallocgc`'s
        // `gcTrigger.test`), not only when the heap grows, because most refills reuse swept
        // spans.  Polling only requests a collection: it never blocks here.
        if VM::VMActivePlan::is_mutator(tls) {
            self.get_gc_trigger().poll(false, Some(self));
        }
        if let Some(hook) = self.refill_hook.get() {
            hook(tls, bytes);
        }
    }

    /// Get a span with free objects for an allocator to cache (Go: `mcentral.cacheSpan`,
    /// `runtime/mcentral.go`): partial swept spans first, then partial unswept spans (swept
    /// here), then full unswept spans (swept here), then a new span. Sets the span's
    /// `sweepgen` to `h + 3` and charges `heap_live`.
    pub(crate) fn cache_span(
        &'static self,
        tls: VMThread,
        spc: SpanClass,
        alloc_options: AllocationOptions,
    ) -> Option<&'static Span> {
        // Go: if we sweep spanBudget spans without finding any free space, just allocate a fresh
        // span.
        let mut span_budget: isize = 100;
        let mut sg = self.sweepgen.load(Ordering::Acquire);

        let found: Option<&'static Span> = 'have: {
            // Try partial swept spans first.
            if let Some(id) = self.central.pop_partial_swept(spc, sg) {
                break 'have Some(self.spans.get(id));
            }
            // Now try partial unswept spans.
            while span_budget >= 0 {
                let Some(id) = self.central.pop_partial_unswept(spc, sg) else {
                    break;
                };
                let s = self.spans.get(id);
                if try_acquire(&s.sweepgen, sg) {
                    // We got ownership of the span, so let's sweep it and use it.
                    self.sweep_span(id, true);
                    break 'have Some(s);
                }
                // Being or already swept by someone else, who put it on the right list.
                span_budget -= 1;
            }
            // Now try full unswept spans, sweeping them and putting them into the right list if
            // we fail to get a span.
            while span_budget >= 0 {
                let Some(id) = self.central.pop_full_unswept(spc, sg) else {
                    break;
                };
                let s = self.spans.get(id);
                if try_acquire(&s.sweepgen, sg) {
                    self.sweep_span(id, true);
                    let free_index = s.next_free_index();
                    if free_index != s.nelems() {
                        s.freeindex.store(free_index, Ordering::Relaxed);
                        break 'have Some(s);
                    }
                    // Sweeping did not give us any free space.
                    self.central.push_full_swept(spc, sg, id);
                }
                span_budget -= 1;
            }
            None
        };

        let span = match found {
            Some(s) => s,
            None => {
                // We failed to get a span from the central sets, so get one from the heap.
                let (_, s) = self.alloc_span(tls, spc, alloc_options)?;
                // Acquiring pages may block for a GC cycle (an allocation stall), which advances
                // `sweepgen`.  The new span belongs to the current generation.
                sg = self.sweepgen.load(Ordering::Acquire);
                s
            }
        };

        assert!(
            span.alloc_count() < span.nelems() && span.freeindex() < span.nelems(),
            "span has no free objects"
        );
        span.init_alloc_cache_at_freeindex();

        // Indicate that this span is cached and prevent asynchronous sweeping in the next sweep
        // phase (Go: `mcache.refill`).
        span.sweepgen.store(sg.wrapping_add(3), Ordering::Release);
        span.alloc_count_before_cache
            .store(span.alloc_count(), Ordering::Relaxed);
        // Charge heap_live as if every free slot will be used (Go: `mcache.refill`).
        let used = span.alloc_count() * span.elemsize();
        self.heap_live
            .fetch_add((span.bytes() - used) as isize, Ordering::Relaxed);
        Some(span)
    }

    /// Return a span from an allocator (Go: `mcentral.uncacheSpan` plus the `heapLive` fix-up of
    /// `mcache.releaseAll`).
    pub(crate) fn uncache_span(&self, span: &Span) {
        let sg = self.sweepgen.load(Ordering::Acquire);
        let stale = span.sweepgen() == sg.wrapping_add(1);
        debug_assert!(stale || span.sweepgen() == sg.wrapping_add(3));
        if !stale {
            // Refill conservatively counted unallocated slots in heap_live. Undo this. (A stale
            // span was cached before heap_live was recomputed at mark termination.)
            let unused = (span.nelems() - span.alloc_count()) * span.elemsize();
            self.heap_live.fetch_sub(unused as isize, Ordering::Relaxed);
        }
        let id = self.span_id_of(span);
        let spc = span.spanclass();
        if stale {
            // Cached before sweep began. It's our responsibility to sweep it.
            span.sweepgen.store(sg.wrapping_sub(1), Ordering::Release);
            self.sweep_span(id, false);
        } else {
            // Indicate that the span is no longer cached.
            span.sweepgen.store(sg, Ordering::Release);
            if span.nelems() > span.alloc_count() {
                self.central.push_partial_swept(spc, sg, id);
            } else {
                self.central.push_full_swept(spc, sg, id);
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // Sweeping (Go: mgcsweep.go)
    // ---------------------------------------------------------------------------------------

    /// Sweep a span whose `sweepgen` the caller has moved to `h - 1` (Go:
    /// `sweepLocked.sweep`). If `preserve` is false, the span is freed if empty, or else put on
    /// the swept set it belongs to. If `preserve` is true, the caller keeps it. Returns whether
    /// the span was freed.
    fn sweep_span(&self, id: SpanId, preserve: bool) -> bool {
        let span = self.spans.get(id);
        let sg = self.sweepgen.load(Ordering::Acquire);
        assert!(
            span.is_in_use() && span.sweepgen() == sg.wrapping_sub(1),
            "SpanSpace sweep: bad span state: in_use = {}, sweepgen = {}, heap sweepgen = {}",
            span.is_in_use(),
            span.sweepgen(),
            sg
        );
        self.pages_swept.fetch_add(span.npages(), Ordering::Relaxed);

        let (nalloc, nfreed) = span.sweep_bits(|_index| {
            // Clear the VO bits of freed objects. We do not know where in the cell the object
            // reference is, so clear the whole cell.
            #[cfg(feature = "vo_bit")]
            crate::util::metadata::vo_bit::bzero_vo_bit(
                span.object_address(_index),
                span.elemsize(),
            );
        });

        span.sweepgen.store(sg, Ordering::Release);
        if nfreed > 0 {
            span.needzero.store(true, Ordering::Relaxed);
        }
        if !preserve {
            let spc = span.spanclass();
            if nalloc == 0 {
                self.free_span(id, span);
                return true;
            }
            if nalloc == span.nelems() {
                self.central.push_full_swept(spc, sg, id);
            } else {
                self.central.push_partial_swept(spc, sg, id);
            }
        }
        false
    }

    /// Every in-use span is swept (`h`) or swept and cached (`h + 3`).
    #[cfg(debug_assertions)]
    fn assert_all_swept(&self) {
        let h = self.sweepgen.load(Ordering::Acquire);
        for (id, span) in self.spans.iter_in_use() {
            let state = sweep_state(span.sweepgen(), h);
            assert!(
                matches!(state, SweepState::Swept | SweepState::Cached),
                "span {id} at {} is {state:?} at sweep termination",
                span.start()
            );
        }
    }
}
