use atomic::Ordering;

use crate::plan::ObjectQueue;
use crate::plan::VectorObjectQueue;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::alloc::allocator::AllocationOptions;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::heap::{FreeListPageResource, PageResource};
use crate::util::metadata;
use crate::util::object_enum::ClosureObjectEnumerator;
use crate::util::object_enum::ObjectEnumerator;
use crate::util::opaque_pointer::*;
use crate::util::treadmill::TreadMill;
use crate::util::{Address, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;

#[allow(unused)]
const PAGE_MASK: usize = !(BYTES_IN_PAGE - 1);
const MARK_BIT: u8 = 0b01;
const NURSERY_BIT: u8 = 0b10;
const LOS_BIT_MASK: u8 = 0b11;

/// This type implements a policy for large objects. Each instance corresponds
/// to one Treadmill space.
pub struct LargeObjectSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    mark_state: u8,
    in_nursery_gc: bool,
    treadmill: TreadMill,
    clear_log_bit_on_sweep: bool,
}

impl<VM: VMBinding> SFT for LargeObjectSpace<VM> {
    fn name(&self) -> &'static str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        self.test_mark_bit(object, self.mark_state)
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

    fn initialize_object_metadata(&self, object: ObjectReference) {
        // VO bit: Set for all objects.
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit(object);
        #[cfg(all(feature = "vo_bit", debug_assertions))]
        {
            use crate::util::constants::LOG_BYTES_IN_PAGE;
            let vo_addr = object.to_raw_address();
            let offset_from_page_start = vo_addr & ((1 << LOG_BYTES_IN_PAGE) - 1) as usize;
            debug_assert!(
                offset_from_page_start < crate::util::metadata::vo_bit::VO_BIT_WORD_TO_REGION,
                "The raw address of ObjectReference is not in the first 512 bytes of a page. The internal pointer searching for LOS won't work."
            );
        }

        let allocate_as_live = self.should_allocate_as_live();
        let into_nursery = !allocate_as_live;

        // mark/nursery bits: Set mark state plus optionally nursery bit.
        {
            let mark_nursery_state = if into_nursery {
                self.mark_state | NURSERY_BIT
            } else {
                self.mark_state
            };

            VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC.store_atomic::<VM, u8>(
                object,
                mark_nursery_state,
                None,
                Ordering::SeqCst,
            );
        }

        // global unlog bit: Set if `unlog_allocated_object`.  Ensure it is not set otherwise.
        if self.common.unlog_allocated_object {
            debug_assert!(self.common.needs_log_bit);
            debug_assert!(
                !allocate_as_live,
                "Currently only ConcurrentImmix can allocate as live, and it doesn't unlog allocated objects in LOS."
            );

            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
        } else {
            #[cfg(debug_assertions)]
            if self.common.needs_log_bit {
                debug_assert_eq!(
                    VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.load_atomic::<VM, u8>(
                        object,
                        None,
                        Ordering::Acquire
                    ),
                    0
                );
            }
        }

        // Add to the treadmill.  Nursery and mature objects need to be added to different sets.
        self.treadmill.add_to_treadmill(object, into_nursery);
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
        use crate::{util::metadata::vo_bit, MMAPPER};

        let mmap_granularity = MMAPPER.granularity();

        // We need to check if metadata address is mapped or not.  But we make use of the granularity of
        // the `Mmapper` to reduce the number of checks.  This records the start of a grain that is
        // tested to be mapped.
        let mut mapped_grain = Address::MAX;

        // For large object space, it is a bit special. We only need to check VO bit for each page.
        let mut cur_page = ptr.align_down(BYTES_IN_PAGE);
        let low_page = ptr
            .saturating_sub(max_search_bytes)
            .align_down(BYTES_IN_PAGE);
        while cur_page >= low_page {
            if cur_page < mapped_grain {
                if !cur_page.is_mapped() {
                    // If the page start is not mapped, there can't be an object in it.
                    return None;
                }
                // This is mapped. No need to check for this chunk.
                mapped_grain = cur_page.align_down(mmap_granularity);
            }
            // For performance, we only check the first word which maps to the first 512 bytes in the page.
            // In almost all the cases, it should be sufficient.
            // However, if the raw address of ObjectReference is not in the first 512 bytes, this won't work.
            // We assert this when we set VO bit for LOS.
            if vo_bit::get_raw_vo_bit_word(cur_page) != 0 {
                // Find the exact address that has vo bit set
                for offset in 0..vo_bit::VO_BIT_WORD_TO_REGION {
                    let addr = cur_page + offset;
                    if unsafe { vo_bit::is_vo_addr(addr) } {
                        return vo_bit::is_internal_ptr_from_vo_bit::<VM>(addr, ptr);
                    }
                }
                unreachable!(
                    "We found vo bit in the raw word, but we cannot find the exact address"
                );
            }

            cur_page -= BYTES_IN_PAGE;
        }
        None
    }
    fn sft_trace_object(
        &self,
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }

    fn debug_print_object_info(&self, object: ObjectReference) {
        println!("marked = {}", self.test_mark_bit(object, self.mark_state));
        println!("nursery = {}", self.is_in_nursery(object));
        self.common.debug_print_object_global_info(object);
    }
}

impl<VM: VMBinding> Space<VM> for LargeObjectSpace<VM> {
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

    fn release_multiple_pages(&mut self, start: Address) {
        self.pr.release_pages(start);
    }

    fn enumerate_objects(&self, enumerator: &mut dyn ObjectEnumerator) {
        // `MMTK::enumerate_objects` is not allowed during GC, so the collection nursery and the
        // from space must be empty.  In `ConcurrentImmix`, mutators may run during GC and call
        // `MMTK::enumerate_objects`.  It has undefined behavior according to the current API, so
        // the assertion failure is expected.
        assert!(
            self.treadmill.is_collect_nursery_empty(),
            "Collection nursery is not empty"
        );
        assert!(
            self.treadmill.is_from_space_empty(),
            "From-space is not empty"
        );

        // Visit objects in the allocation nursery and the to-space, which contain young and old
        // objects, respectively, during mutator time.
        self.treadmill.enumerate_objects(enumerator, false);
    }

    fn clear_side_log_bits(&self) {
        let mut enumerator = ClosureObjectEnumerator::<_, VM>::new(|object| {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.clear::<VM>(object, Ordering::SeqCst);
        });
        // Visit all objects.  It can be ordered arbitrarily with `Self::Release` which sweeps dead
        // objects (removing them from the treadmill) and clears their unlog bits, too.
        self.treadmill.enumerate_objects(&mut enumerator, true);
    }

    fn set_side_log_bits(&self) {
        let mut enumerator = ClosureObjectEnumerator::<_, VM>::new(|object| {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
        });
        // Visit all objects.
        self.treadmill.enumerate_objects(&mut enumerator, true);
    }
}

use crate::scheduler::GCWorker;
use crate::util::copy::CopySemantics;

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for LargeObjectSpace<VM> {
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }
    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        false
    }
}

impl<VM: VMBinding> LargeObjectSpace<VM> {
    pub fn new(
        args: crate::policy::space::PlanCreateSpaceArgs<VM>,
        protect_memory_on_release: bool,
        clear_log_bit_on_sweep: bool,
    ) -> Self {
        let is_discontiguous = args.vmrequest.is_discontiguous();
        let vm_map = args.vm_map;
        let common = CommonSpace::new(args.into_policy_args(
            false,
            false,
            metadata::extract_side_metadata(&[*VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC]),
        ));
        let mut pr = if is_discontiguous {
            FreeListPageResource::new_discontiguous(vm_map)
        } else {
            FreeListPageResource::new_contiguous(common.start, common.extent, vm_map)
        };
        pr.protect_memory_on_release = if protect_memory_on_release {
            Some(common.mmap_strategy().prot)
        } else {
            None
        };
        LargeObjectSpace {
            pr,
            common,
            mark_state: 0,
            in_nursery_gc: false,
            treadmill: TreadMill::new(),
            clear_log_bit_on_sweep,
        }
    }

    pub fn prepare(&mut self, full_heap: bool) {
        if full_heap {
            self.mark_state = MARK_BIT - self.mark_state;
        }
        self.treadmill.flip(full_heap);
        self.in_nursery_gc = !full_heap;
    }

    pub fn release(&mut self, full_heap: bool) {
        // We swapped the allocation nursery and the collection nursery when GC starts, and we don't
        // add objects to the allocation nursery during GC.  It should have remained empty during
        // the whole GC.
        debug_assert!(self.treadmill.is_alloc_nursery_empty());

        self.sweep_large_pages(true);
        debug_assert!(self.treadmill.is_collect_nursery_empty());
        if full_heap {
            self.sweep_large_pages(false);
            debug_assert!(self.treadmill.is_from_space_empty());
        }
    }

    // Allow nested-if for this function to make it clear that test_and_mark() is only executed
    // for the outer condition is met.
    #[allow(clippy::collapsible_if)]
    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "vo_bit")]
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set(object),
            "{:x}: VO bit not set",
            object
        );
        let nursery_object = self.is_in_nursery(object);
        trace!(
            "LOS object {} {} a nursery object",
            object,
            if nursery_object { "is" } else { "is not" }
        );
        if !self.in_nursery_gc || nursery_object {
            // Note that test_and_mark() has side effects of
            // clearing nursery bit/moving objects out of logical nursery
            if self.test_and_mark(object, self.mark_state) {
                trace!("LOS object {} is being marked now", object);
                self.treadmill.copy(object, nursery_object);
                // We just moved the object out of the logical nursery, mark it as unlogged.
                // We also unlog mature objects as their unlog bit may have been unset before the
                // full-heap GC
                if self.common.unlog_traced_object {
                    VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                        .mark_as_unlogged::<VM>(object, Ordering::SeqCst);
                }
                queue.enqueue(object);
            } else {
                trace!(
                    "LOS object {} is not being marked now, it was marked before",
                    object
                );
            }
        }
        object
    }

    fn sweep_large_pages(&mut self, sweep_nursery: bool) {
        let sweep = |object: ObjectReference| {
            #[cfg(feature = "vo_bit")]
            crate::util::metadata::vo_bit::unset_vo_bit(object);
            // Clear log bits for dead objects to prevent a new nursery object having the unlog bit set
            if self.clear_log_bit_on_sweep {
                VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.clear::<VM>(object, Ordering::SeqCst);
            }
            self.pr
                .release_pages(get_super_page(object.to_object_start::<VM>()));
        };
        if sweep_nursery {
            for object in self.treadmill.collect_nursery() {
                sweep(object);
            }
        } else {
            for object in self.treadmill.collect_mature() {
                sweep(object)
            }
        }
    }

    /// Enumerate objects in the to-space.  It is a workaround for Compressor which currently needs
    /// to enumerate reachable objects for during reference forwarding.
    pub(crate) fn enumerate_to_space_objects(&self, enumerator: &mut dyn ObjectEnumerator) {
        // This function is intended to enumerate objects in the to-space.
        // The alloc nursery should have remained empty during the GC.
        debug_assert!(self.treadmill.is_alloc_nursery_empty());
        // We only need to visit the to_space, which contains all objects determined to be live.
        self.treadmill.enumerate_objects(enumerator, false);
    }

    /// Allocate an object
    pub fn allocate_pages(
        &self,
        tls: VMThread,
        pages: usize,
        alloc_options: AllocationOptions,
    ) -> Address {
        self.acquire(tls, pages, alloc_options)
    }

    /// Test if the object's mark bit is the same as the given value. If it is not the same,
    /// the method will attemp to mark the object and clear its nursery bit. If the attempt
    /// succeeds, the method will return true, meaning the object is marked by this invocation.
    /// Otherwise, it returns false.
    fn test_and_mark(&self, object: ObjectReference, value: u8) -> bool {
        loop {
            let mask = if self.in_nursery_gc {
                LOS_BIT_MASK
            } else {
                MARK_BIT
            };
            let old_value = VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC.load_atomic::<VM, u8>(
                object,
                None,
                Ordering::SeqCst,
            );
            let mark_bit = old_value & mask;
            if mark_bit == value {
                return false;
            }
            // using LOS_BIT_MASK have side effects of clearing nursery bit
            if VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    old_value & !LOS_BIT_MASK | value,
                    None,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }
        true
    }

    fn test_mark_bit(&self, object: ObjectReference, value: u8) -> bool {
        VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        ) & MARK_BIT
            == value
    }

    /// Check if a given object is in nursery
    fn is_in_nursery(&self, object: ObjectReference) -> bool {
        VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::Relaxed,
        ) & NURSERY_BIT
            == NURSERY_BIT
    }

    pub fn is_marked(&self, object: ObjectReference) -> bool {
        self.test_mark_bit(object, self.mark_state)
    }
}

fn get_super_page(cell: Address) -> Address {
    cell.align_down(BYTES_IN_PAGE)
}
