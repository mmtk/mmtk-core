use crate::util::address::Address;

use std::sync::atomic::Ordering;

use crate::plan::selected_plan::SelectedPlan;
use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::constants::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::vm::{ActivePlan, Collection};

use downcast_rs::Downcast;

// FIXME: Put this somewhere more appropriate
pub const ALIGNMENT_VALUE: usize = 0xdead_beef;
pub const LOG_MIN_ALIGNMENT: usize = LOG_BYTES_IN_INT as usize;
pub const MIN_ALIGNMENT: usize = 1 << LOG_MIN_ALIGNMENT;
#[cfg(target_arch = "x86")]
pub const MAX_ALIGNMENT_SHIFT: usize = 1 + LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;
#[cfg(target_arch = "x86_64")]
pub const MAX_ALIGNMENT_SHIFT: usize = LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;

pub const MAX_ALIGNMENT: usize = MIN_ALIGNMENT << MAX_ALIGNMENT_SHIFT;

#[inline(always)]
pub fn align_allocation_no_fill(region: Address, alignment: usize, offset: isize) -> Address {
    align_allocation(region, alignment, offset, MIN_ALIGNMENT, false)
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
    // Make sure MIN_ALIGNMENT is reasonable.
    #[allow(clippy::assertions_on_constants)]
    {
        debug_assert!(MIN_ALIGNMENT >= BYTES_IN_INT);
    }
    debug_assert!(!(fillalignmentgap && region.is_zero()));
    debug_assert!(alignment <= MAX_ALIGNMENT);
    debug_assert!(offset >= 0);
    debug_assert!(region.is_aligned_to(MIN_ALIGNMENT));
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
    trace!(
        "size={}, alignment={}, known_alignment={}, MIN_ALIGNMENT={}",
        size,
        alignment,
        known_alignment,
        MIN_ALIGNMENT
    );
    debug_assert!(size == size & !(known_alignment - 1));
    debug_assert!(known_alignment >= MIN_ALIGNMENT);

    if MAX_ALIGNMENT <= MIN_ALIGNMENT || alignment <= known_alignment {
        size
    } else {
        size + alignment - known_alignment
    }
}

pub trait Allocator<VM: VMBinding>: Downcast {
    fn get_tls(&self) -> OpaquePointer;

    fn get_space(&self) -> Option<&'static dyn Space<VM>>;
    fn get_plan(&self) -> &'static SelectedPlan<VM>;

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;

    #[inline(never)]
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow_inline(size, align, offset)
    }

    #[inline(always)]
    fn alloc_slow_inline(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let tls = self.get_tls();

        // Information about the previous collection.
        let mut emergency_collection = false;
        loop {
            // Try to allocate using the slow path
            let result = self.alloc_slow_once(size, align, offset);

            if unsafe { !VM::VMActivePlan::is_mutator(tls) } {
                debug_assert!(!result.is_zero());
                return result;
            }

            let plan = self.get_plan().base();
            if !result.is_zero() {
                // TODO: Check if we need oom lock.
                // It seems the lock only protects access to the atomic boolean. We could possibly do
                // so with compare and swap

                // Report allocation success to assist OutOfMemory handling.
                if !plan.allocation_success.load(Ordering::Relaxed) {
                    // XXX: Can we replace this with:
                    // ALLOCATION_SUCCESS.store(1, Ordering::SeqCst);
                    // (and get rid of the lock)
                    let guard = plan.oom_lock.lock().unwrap();
                    plan.allocation_success.store(true, Ordering::Relaxed);
                    drop(guard);
                }
                return result;
            }

            if emergency_collection {
                trace!("Emergency collection");
                // Report allocation success to assist OutOfMemory handling.
                let guard = plan.oom_lock.lock().unwrap();
                let fail_with_oom = !plan.allocation_success.load(Ordering::Relaxed);
                // This seems odd, but we must allow each OOM to run its course (and maybe give us back memory)
                plan.allocation_success.store(true, Ordering::Relaxed);
                drop(guard);
                trace!("fail with oom={}", fail_with_oom);
                if fail_with_oom {
                    VM::VMCollection::out_of_memory(tls);
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
            //    VMActivePlan::mutator(tls).get_allocator_from_space(space)
            //};

            /*
             * Record whether last collection was an Emergency collection.
             * If so, we make one more attempt to allocate before we signal
             * an OOM.
             */
            emergency_collection = self.get_plan().is_emergency_collection();
            trace!("Got emergency collection as {}", emergency_collection);
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address;
}

impl_downcast!(Allocator<VM> where VM: VMBinding);
