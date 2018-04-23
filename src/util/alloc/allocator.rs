use ::util::address::Address;

use ::policy::space::Space;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use ::util::constants::*;
use ::util::heap::PageResource;
use ::vm::{ActivePlan, VMActivePlan, Collection, VMCollection};
use ::plan::MutatorContext;
use ::plan::selected_plan::PLAN;
use ::plan::selected_plan::SelectedPlan;
use ::plan::Plan;

// FIXME: Put this somewhere more appropriate
pub const ALIGNMENT_VALUE: usize = 0xdeadbeef;
pub const LOG_MIN_ALIGNMENT: usize = LOG_BYTES_IN_INT as usize;
pub const MIN_ALIGNMENT: usize = 1 << LOG_MIN_ALIGNMENT;
#[cfg(target_arch = "x86")]
pub const MAX_ALIGNMENT_SHIFT: usize = 1 + LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;
#[cfg(target_arch = "x86_64")]
pub const MAX_ALIGNMENT_SHIFT: usize = 0 + LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;

pub const MAX_ALIGNMENT: usize = MIN_ALIGNMENT << MAX_ALIGNMENT_SHIFT;

static ALLOCATION_SUCCESS: AtomicBool = AtomicBool::new(false);
static COLLECTION_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
lazy_static! {
    static ref OOM_LOCK: Mutex<()> = Mutex::new(());
}

#[inline(always)]
pub fn align_allocation_no_fill(
    region: Address,
    alignment: usize,
    offset: isize,
) -> Address {
    return align_allocation(
        region,
        alignment,
        offset,
        MIN_ALIGNMENT,
        false,
    );
}

#[inline(always)]
pub fn align_allocation(
    region: Address,
    alignment: usize,
    offset: isize,
    known_alignment: usize,
    fillalignmentgap: bool,
) -> Address {
    debug_assert!(known_alignment >= MIN_ALIGNMENT);
    debug_assert!(MIN_ALIGNMENT >= BYTES_IN_INT);
    debug_assert!(!(fillalignmentgap && region.is_zero()));
    debug_assert!(alignment <= MAX_ALIGNMENT);
    debug_assert!(offset >= 0);
    debug_assert!((
        (region.as_usize() as isize) & ((MIN_ALIGNMENT - 1) as isize)
    ) == 0);
    debug_assert!((alignment & (MIN_ALIGNMENT - 1)) == 0);
    debug_assert!((offset & (MIN_ALIGNMENT - 1) as isize) == 0);

    // No alignment ever required.
    if alignment <= known_alignment || MAX_ALIGNMENT <= MIN_ALIGNMENT {
        return region;
    }

    // May require an alignment
    let region_isize = region.as_usize() as isize;

    let mask = (alignment - 1) as isize; // fromIntSignExtend
    let neg_off = -offset; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    if fillalignmentgap && (ALIGNMENT_VALUE != 0) {
        fill_alignment_gap(region, region + delta);
    }

    region + delta
}

#[inline(always)]
pub fn fill_alignment_gap(immut_start: Address, end: Address) {
    let mut start = immut_start;

    if MAX_ALIGNMENT - MIN_ALIGNMENT == BYTES_IN_INT {
        // At most a single hole
        if end - start != 0 {
            unsafe {
                start.store(ALIGNMENT_VALUE);
            }
        }
    } else {
        while start < end {
            unsafe {
                start.store(ALIGNMENT_VALUE);
            }
            start += BYTES_IN_INT;
        }
    }
}

#[inline(always)]
pub fn get_maximum_aligned_size(size: usize, alignment: usize, known_alignment: usize) -> usize {
    trace!("size={}, alignment={}, known_alignment={}, MIN_ALIGNMENT={}", size, alignment,
           known_alignment, MIN_ALIGNMENT);
    debug_assert!(size == size & !(known_alignment - 1));
    debug_assert!(known_alignment >= MIN_ALIGNMENT);

    if MAX_ALIGNMENT <= MIN_ALIGNMENT || alignment <= known_alignment {
        return size;
    } else {
        return size + alignment - known_alignment;
    }
}

pub fn determine_collection_attempts() -> usize {
    if !ALLOCATION_SUCCESS.load(Ordering::Relaxed) {
        COLLECTION_ATTEMPTS.store(COLLECTION_ATTEMPTS.load(Ordering::Relaxed) + 1,
                                  Ordering::Relaxed);
    } else {
        ALLOCATION_SUCCESS.store(false, Ordering::Relaxed);
        COLLECTION_ATTEMPTS.store(1, Ordering::Relaxed);
    }

    COLLECTION_ATTEMPTS.load(Ordering::Relaxed)
}

pub trait Allocator<S: Space<PR>, PR: PageResource<S>> {
    fn get_thread_id(&self) -> usize;

    fn get_space(&self) -> Option<&'static S>;

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address;

    #[inline(always)]
    fn alloc_slow_inline(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let thread_id = self.get_thread_id();
        let tmp = self.get_space();
        let space = tmp.as_ref().unwrap();

        // Information about the previous collection.
        let mut emergency_collection = false;
        loop {
            // Try to allocate using the slow path
            let result = self.alloc_slow_once(size, align, offset);

            if unsafe { !VMActivePlan::is_mutator(thread_id) } {
                debug_assert!(!result.is_zero());
                return result;
            }

            if !result.is_zero() {
                // Report allocation success to assist OutOfMemory handling.
                if !ALLOCATION_SUCCESS.load(Ordering::Relaxed) {
                    // XXX: Can we replace this with:
                    // ALLOCATION_SUCCESS.store(1, Ordering::SeqCst);
                    // (and get rid of the lock)
                    let guard = OOM_LOCK.lock().unwrap();
                    ALLOCATION_SUCCESS.store(true, Ordering::Relaxed);
                    drop(guard);
                }
                return result;
            }

            if emergency_collection {
                trace!("Emergency collection");
                // Report allocation success to assist OutOfMemory handling.
                let guard = OOM_LOCK.lock().unwrap();
                let fail_with_oom = !ALLOCATION_SUCCESS.load(Ordering::Relaxed);
                // This seems odd, but we must allow each OOM to run its course (and maybe give us back memory)
                ALLOCATION_SUCCESS.store(true, Ordering::Relaxed);
                drop(guard);
                trace!("fail with oom={}", fail_with_oom);
                if fail_with_oom {
                    VMCollection::out_of_memory();
                    trace!("Not reached");
                }
            }

            /* This is in case a GC occurs, and our mutator context is stale.
             * In some VMs the scheduler can change the affinity between the
             * current thread and the mutator context. This is possible for
             * VMs that dynamically multiplex Java threads onto multiple mutator
             * contexts. */
            // FIXME: No good way to do this
            //current = unsafe {
            //    VMActivePlan::mutator(thread_id).get_allocator_from_space(space)
            //};

            /*
             * Record whether last collection was an Emergency collection.
             * If so, we make one more attempt to allocate before we signal
             * an OOM.
             */
            emergency_collection = <SelectedPlan as Plan>::is_emergency_collection();
            trace!("Got emergency collection as {}", emergency_collection);
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address;
}