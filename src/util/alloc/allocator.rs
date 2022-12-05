use crate::util::address::Address;
use std::sync::atomic::Ordering;

use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::constants::*;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::vm::{ActivePlan, Collection};
use downcast_rs::Downcast;

#[repr(C)]
#[derive(Debug)]
/// A list of errors that MMTk can encounter during allocation.
pub enum AllocationError {
    /// The specified heap size is too small for the given program to continue.
    HeapOutOfMemory,
    /// The OS is unable to mmap or acquire more memory. Critical error. MMTk expects the VM to
    /// abort if such an error is thrown.
    MmapOutOfMemory,
}

#[inline(always)]
pub fn align_allocation_no_fill<VM: VMBinding>(
    region: Address,
    alignment: usize,
    offset: isize,
) -> Address {
    align_allocation_inner::<VM>(region, alignment, offset, VM::MIN_ALIGNMENT, false)
}

#[inline(always)]
pub fn align_allocation<VM: VMBinding>(
    region: Address,
    alignment: usize,
    offset: isize,
) -> Address {
    align_allocation_inner::<VM>(region, alignment, offset, VM::MIN_ALIGNMENT, true)
}

#[inline(always)]
pub fn align_allocation_inner<VM: VMBinding>(
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
pub fn get_maximum_aligned_size<VM: VMBinding>(size: usize, alignment: usize) -> usize {
    get_maximum_aligned_size_inner::<VM>(size, alignment, VM::MIN_ALIGNMENT)
}

#[inline(always)]
pub fn get_maximum_aligned_size_inner<VM: VMBinding>(
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

/// A trait which implements allocation routines. Every allocator needs to implements this trait.
pub trait Allocator<VM: VMBinding>: Downcast {
    /// Return the [`VMThread`] associated with this allocator instance.
    fn get_tls(&self) -> VMThread;

    /// Return the [`Space`](src/policy/space/Space) instance associated with this allocator instance.
    fn get_space(&self) -> &'static dyn Space<VM>;

    /// Return the [`Plan`] instance that this allocator instance is associated with.
    fn get_plan(&self) -> &'static dyn Plan<VM = VM>;

    /// Return if this allocator can do thread local allocation. If an allocator does not do thread
    /// local allocation, each allocation will go to slowpath and will have a check for GC polls.
    fn does_thread_local_allocation(&self) -> bool;

    /// Return at which granularity the allocator acquires memory from the global space and use
    /// them as thread local buffer. For example, the [`BumpAllocator`](crate::util::alloc::BumpAllocator) acquires memory at 32KB
    /// blocks. Depending on the actual size for the current object, they always acquire memory of
    /// N*32KB (N>=1). Thus the [`BumpAllocator`](crate::util::alloc::BumpAllocator) returns 32KB for this method.  Only allocators
    /// that do thread local allocation need to implement this method.
    fn get_thread_local_buffer_granularity(&self) -> usize {
        assert!(self.does_thread_local_allocation(), "An allocator that does not thread local allocation does not have a buffer granularity.");
        unimplemented!()
    }

    /// An allocation attempt. The implementation of this function depends on the allocator used.
    /// If an allocator supports thread local allocations, then the allocation will be serviced
    /// from its TLAB, otherwise it will default to using the slowpath, i.e. [`alloc_slow`](Allocator::alloc_slow).
    ///
    /// Note that in the case where the VM is out of memory, we invoke
    /// [`Collection::out_of_memory`] to inform the binding and then return a null pointer back to
    /// it. We have no assumptions on whether the VM will continue executing or abort immediately.
    ///
    /// An allocator needs to make sure the object reference for the returned address is in the same
    /// chunk as the returned address (so the side metadata and the SFT for an object reference is valid).
    /// See [`crate::util::alloc::object_ref_guard`](util/alloc/object_ref_guard).
    ///
    /// Arguments:
    /// * `size`: the allocation size in bytes.
    /// * `align`: the required alignment in bytes.
    /// * `offset` the required offset in bytes.
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;

    /// Slowpath allocation attempt. This function is explicitly not inlined for performance
    /// considerations.
    ///
    /// Arguments:
    /// * `size`: the allocation size in bytes.
    /// * `align`: the required alignment in bytes.
    /// * `offset` the required offset in bytes.
    #[inline(never)]
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow_inline(size, align, offset)
    }

    /// Slowpath allocation attempt. This function executes the actual slowpath allocation.  A
    /// slowpath allocation in MMTk attempts to allocate the object using the per-allocator
    /// definition of [`alloc_slow_once`](Allocator::alloc_slow_once). This function also accounts for increasing the
    /// allocation bytes in order to support stress testing. In case precise stress testing is
    /// being used, the [`alloc_slow_once_precise_stress`](Allocator::alloc_slow_once_precise_stress) function is used instead.
    ///
    /// Note that in the case where the VM is out of memory, we invoke
    /// [`Collection::out_of_memory`] with a [`AllocationError::HeapOutOfMemory`] error to inform
    /// the binding and then return a null pointer back to it. We have no assumptions on whether
    /// the VM will continue executing or abort immediately on a
    /// [`AllocationError::HeapOutOfMemory`] error.
    ///
    /// Arguments:
    /// * `size`: the allocation size in bytes.
    /// * `align`: the required alignment in bytes.
    /// * `offset` the required offset in bytes.
    #[inline(always)]
    fn alloc_slow_inline(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let tls = self.get_tls();
        let plan = self.get_plan().base();
        let is_mutator = VM::VMActivePlan::is_mutator(tls);
        let stress_test = plan.is_stress_test_gc_enabled();

        // Information about the previous collection.
        let mut emergency_collection = false;
        let mut previous_result_zero = false;

        loop {
            // Try to allocate using the slow path
            let result = if is_mutator && stress_test && plan.is_precise_stress() {
                // If we are doing precise stress GC, we invoke the special allow_slow_once call.
                // alloc_slow_once_precise_stress() should make sure that every allocation goes
                // to the slowpath (here) so we can check the allocation bytes and decide
                // if we need to do a stress GC.

                // If we should do a stress GC now, we tell the alloc_slow_once_precise_stress()
                // so they would avoid try any thread local allocation, and directly call
                // global acquire and do a poll.
                let need_poll = is_mutator && plan.should_do_stress_gc();
                self.alloc_slow_once_precise_stress(size, align, offset, need_poll)
            } else {
                // If we are not doing precise stress GC, just call the normal alloc_slow_once().
                // Normal stress test only checks for stress GC in the slowpath.
                self.alloc_slow_once(size, align, offset)
            };

            if !is_mutator {
                debug_assert!(!result.is_zero());
                return result;
            }

            if !result.is_zero() {
                // Report allocation success to assist OutOfMemory handling.
                if !plan.allocation_success.load(Ordering::Relaxed) {
                    plan.allocation_success.store(true, Ordering::SeqCst);
                }

                // Only update the allocation bytes if we haven't failed a previous allocation in this loop
                if stress_test && self.get_plan().is_initialized() && !previous_result_zero {
                    let allocated_size =
                        if plan.is_precise_stress() || !self.does_thread_local_allocation() {
                            // For precise stress test, or for allocators that do not have thread local buffer,
                            // we know exactly how many bytes we allocate.
                            size
                        } else {
                            // For normal stress test, we count the entire thread local buffer size as allocated.
                            crate::util::conversions::raw_align_up(
                                size,
                                self.get_thread_local_buffer_granularity(),
                            )
                        };
                    let _allocation_bytes = plan.increase_allocation_bytes_by(allocated_size);

                    // This is the allocation hook for the analysis trait. If you want to call
                    // an analysis counter specific allocation hook, then here is the place to do so
                    #[cfg(feature = "analysis")]
                    if _allocation_bytes > *plan.options.analysis_factor {
                        trace!(
                            "Analysis: allocation_bytes = {} more than analysis_factor = {}",
                            _allocation_bytes,
                            *plan.options.analysis_factor
                        );

                        plan.analysis_manager.alloc_hook(size, align, offset);
                    }
                }

                return result;
            }

            // It is possible to have cases where a thread is blocked for another GC (non emergency)
            // immediately after being blocked for a GC (emergency) (e.g. in stress test), that is saying
            // the thread does not leave this loop between the two GCs. The local var 'emergency_collection'
            // was set to true after the first GC. But when we execute this check below, we just finished
            // the second GC, which is not emergency. In such case, we will give a false OOM.
            // We cannot just rely on the local var. Instead, we get the emergency collection value again,
            // and check both.
            if emergency_collection && self.get_plan().is_emergency_collection() {
                trace!("Emergency collection");
                // Report allocation success to assist OutOfMemory handling.
                // This seems odd, but we must allow each OOM to run its course (and maybe give us back memory)
                let fail_with_oom = !plan.allocation_success.swap(true, Ordering::SeqCst);
                trace!("fail with oom={}", fail_with_oom);
                if fail_with_oom {
                    // Note that we throw a `HeapOutOfMemory` error here and return a null ptr back to the VM
                    trace!("Throw HeapOutOfMemory!");
                    VM::VMCollection::out_of_memory(tls, AllocationError::HeapOutOfMemory);
                    plan.allocation_success.swap(false, Ordering::SeqCst);
                    return result;
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

            // Record whether last collection was an Emergency collection. If so, we make one more
            // attempt to allocate before we signal an OOM.
            emergency_collection = self.get_plan().is_emergency_collection();
            trace!("Got emergency collection as {}", emergency_collection);
            previous_result_zero = true;
        }
    }

    /// Single slow path allocation attempt. This is called by [`alloc_slow_inline`](Allocator::alloc_slow_inline). The
    /// implementation of this function depends on the allocator used. Generally, if an allocator
    /// supports thread local allocations, it will try to allocate more TLAB space here. If it
    /// doesn't, then (generally) the allocator simply allocates enough space for the current
    /// object.
    ///
    /// Arguments:
    /// * `size`: the allocation size in bytes.
    /// * `align`: the required alignment in bytes.
    /// * `offset` the required offset in bytes.
    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address;

    /// Single slowpath allocation attempt for stress test. When the stress factor is set (e.g. to
    /// N), we would expect for every N bytes allocated, we will trigger a stress GC.  However, for
    /// allocators that do thread local allocation, they may allocate from their thread local
    /// buffer which does not have a GC poll check, and they may even allocate with the JIT
    /// generated allocation fastpath which is unaware of stress test GC. For both cases, we are
    /// not able to guarantee a stress GC is triggered every N bytes. To solve this, when the
    /// stress factor is set, we will call this method instead of the normal alloc_slow_once(). We
    /// expect the implementation of this slow allocation will trick the fastpath so every
    /// allocation will fail in the fastpath, jump to the slow path and eventually call this method
    /// again for the actual allocation.
    ///
    /// The actual implementation about how to trick the fastpath may vary. For example, our bump
    /// pointer allocator will set the thread local buffer limit to the buffer size instead of the
    /// buffer end address. In this case, every fastpath check (cursor + size < limit) will fail,
    /// and jump to this slowpath. In the slowpath, we still allocate from the thread local buffer,
    /// and recompute the limit (remaining buffer size).
    ///
    /// If an allocator does not do thread local allocation (which returns false for
    /// does_thread_local_allocation()), it does not need to override this method. The default
    /// implementation will simply call allow_slow_once() and it will work fine for allocators that
    /// do not have thread local allocation.
    ///
    /// Arguments:
    /// * `size`: the allocation size in bytes.
    /// * `align`: the required alignment in bytes.
    /// * `offset` the required offset in bytes.
    /// * `need_poll`: if this is true, the implementation must poll for a GC, rather than
    ///   attempting to allocate from the local buffer.
    fn alloc_slow_once_precise_stress(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        need_poll: bool,
    ) -> Address {
        // If an allocator does thread local allocation but does not override this method to
        // provide a correct implementation, we will log a warning.
        if self.does_thread_local_allocation() && need_poll {
            warn!("{} does not support stress GC (An allocator that does thread local allocation needs to implement allow_slow_once_stress_test()).", std::any::type_name::<Self>());
        }
        self.alloc_slow_once(size, align, offset)
    }

    /// The [`crate::plan::Mutator`] that includes this allocator is going to be destroyed. Some allocators
    /// may need to save/transfer its thread local data to the space.
    fn on_mutator_destroy(&mut self) {
        // By default, do nothing
    }
}

impl_downcast!(Allocator<VM> where VM: VMBinding);
