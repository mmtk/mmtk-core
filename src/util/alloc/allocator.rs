use crate::util::address::Address;
use crate::util::constants::DEFAULT_STRESS_FACTOR;

use std::sync::atomic::Ordering;

use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::constants::*;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::vm::{ActivePlan, Collection};
use downcast_rs::Downcast;

#[inline(always)]
pub fn align_allocation_no_fill<VM: VMBinding>(
    region: Address,
    alignment: usize,
    offset: isize,
) -> Address {
    align_allocation::<VM>(region, alignment, offset, VM::MIN_ALIGNMENT, false)
}

#[inline(always)]
pub fn align_allocation<VM: VMBinding>(
    region: Address,
    alignment: usize,
    offset: isize,
    known_alignment: usize,
    fillalignmentgap: bool,
) -> Address {
    debug_assert!(known_alignment >= VM::MIN_ALIGNMENT);
    // Make sure MIN_ALIGNMENT is reasonable.
    #[allow(clippy::assertions_on_constants)]
    {
        debug_assert!(VM::MIN_ALIGNMENT >= BYTES_IN_INT);
    }
    debug_assert!(!(fillalignmentgap && region.is_zero()));
    debug_assert!(alignment <= VM::MAX_ALIGNMENT);
    debug_assert!(offset >= 0);
    debug_assert!(region.is_aligned_to(VM::ALLOC_END_ALIGNMENT));
    debug_assert!((alignment & (VM::MIN_ALIGNMENT - 1)) == 0);
    debug_assert!((offset & (VM::MIN_ALIGNMENT - 1) as isize) == 0);

    // No alignment ever required.
    if alignment <= known_alignment || VM::MAX_ALIGNMENT <= VM::MIN_ALIGNMENT {
        return region;
    }

    // May require an alignment
    let region_isize = region.as_usize() as isize;

    let mask = (alignment - 1) as isize; // fromIntSignExtend
    let neg_off = -offset; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    if fillalignmentgap && (VM::ALIGNMENT_VALUE != 0) {
        fill_alignment_gap::<VM>(region, region + delta);
    }

    region + delta
}

#[inline(always)]
pub fn fill_alignment_gap<VM: VMBinding>(immut_start: Address, end: Address) {
    let mut start = immut_start;

    if VM::MAX_ALIGNMENT - VM::MIN_ALIGNMENT == BYTES_IN_INT {
        // At most a single hole
        if end - start != 0 {
            unsafe {
                start.store(VM::ALIGNMENT_VALUE);
            }
        }
    } else {
        while start < end {
            unsafe {
                start.store(VM::ALIGNMENT_VALUE);
            }
            start += BYTES_IN_INT;
        }
    }
}

#[inline(always)]
pub fn get_maximum_aligned_size<VM: VMBinding>(
    size: usize,
    alignment: usize,
    known_alignment: usize,
) -> usize {
    trace!(
        "size={}, alignment={}, known_alignment={}, MIN_ALIGNMENT={}",
        size,
        alignment,
        known_alignment,
        VM::MIN_ALIGNMENT
    );
    debug_assert!(size == size & !(known_alignment - 1));
    debug_assert!(known_alignment >= VM::MIN_ALIGNMENT);

    if VM::MAX_ALIGNMENT <= VM::MIN_ALIGNMENT || alignment <= known_alignment {
        size
    } else {
        size + alignment - known_alignment
    }
}

pub trait Allocator<VM: VMBinding>: Downcast {
    fn get_tls(&self) -> VMThread;

    fn get_space(&self) -> &'static dyn Space<VM>;
    fn get_plan(&self) -> &'static dyn Plan<VM = VM>;

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;

    #[inline(never)]
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow_inline(size, align, offset)
    }

    #[inline(always)]
    fn alloc_slow_inline(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let tls = self.get_tls();
        let plan = self.get_plan().base();
        let stress_test = plan.options.stress_factor != DEFAULT_STRESS_FACTOR
            || plan.options.analysis_factor != DEFAULT_STRESS_FACTOR;

        // Information about the previous collection.
        let mut emergency_collection = false;
        let mut previous_result_zero = false;
        loop {
            // Try to allocate using the slow path
            let result = self.alloc_slow_once(size, align, offset);

            if !unsafe { VM::VMActivePlan::is_mutator(tls) } {
                debug_assert!(!result.is_zero());
                return result;
            }

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

                // When a GC occurs, the resultant address provided by `acquire()` is 0x0.
                // Hence, another iteration of this loop occurs. In such a case, the second
                // iteration tries to allocate again, and if is successful, then the allocation
                // bytes are updated. However, this leads to double counting of the allocation:
                // (i) by the original alloc_slow_inline(); and (ii) by the alloc_slow_inline()
                // called by acquire(). In order to not double count the allocation, we only
                // update allocation bytes if the previous result wasn't 0x0.
                if stress_test && self.get_plan().is_initialized() && !previous_result_zero {
                    plan.increase_allocation_bytes_by(size);
                }

                return result;
            }

            // It is possible to have cases where a thread is blocked for another GC (non emergency)
            // immediately after being blocked for a GC (emergency) (e.g. in stress test), that is saying the thread does not
            // leave this loop between the two GCs. The local var 'emergency_collection' was set to true
            // after the first GC. But when we execute this check below, we just finished the second GC,
            // which is not emergency. In such case, we will give a false OOM.
            // We cannot just rely on the local var. Instead, we get the emergency collection value again,
            // and check both.
            if emergency_collection && self.get_plan().is_emergency_collection() {
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
            previous_result_zero = true;
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address;
}

impl_downcast!(Allocator<VM> where VM: VMBinding);
