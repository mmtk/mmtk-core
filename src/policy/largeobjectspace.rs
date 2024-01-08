use atomic::Ordering;

use crate::plan::ObjectQueue;
use crate::plan::VectorObjectQueue;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::heap::{FreeListPageResource, PageResource};
use crate::util::metadata;
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
}

impl<VM: VMBinding> SFT for LargeObjectSpace<VM> {
    fn name(&self) -> &str {
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
    fn initialize_object_metadata(&self, object: ObjectReference, alloc: bool) {
        let old_value = VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        let mut new_value = (old_value & (!LOS_BIT_MASK)) | self.mark_state;
        if alloc {
            new_value |= NURSERY_BIT;
        }
        VM::VMObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC.store_atomic::<VM, u8>(
            object,
            new_value,
            None,
            Ordering::SeqCst,
        );

        // If this object is freshly allocated, we do not set it as unlogged
        if !alloc && self.common.needs_log_bit {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
        }

        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit::<VM>(object);
        self.treadmill.add_to_treadmill(object, alloc);
    }
    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr::<VM>(addr).is_some()
    }
    fn sft_trace_object(
        &self,
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        self.trace_object(queue, object)
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

    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        self.common().initialize_sft(self.as_sft(), sft_map)
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, start: Address) {
        self.pr.release_pages(start);
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
        pr.protect_memory_on_release = protect_memory_on_release;
        LargeObjectSpace {
            pr,
            common,
            mark_state: 0,
            in_nursery_gc: false,
            treadmill: TreadMill::new(),
        }
    }

    pub fn prepare(&mut self, full_heap: bool) {
        if full_heap {
            debug_assert!(self.treadmill.is_from_space_empty());
            self.mark_state = MARK_BIT - self.mark_state;
        }
        self.treadmill.flip(full_heap);
        self.in_nursery_gc = !full_heap;
    }

    pub fn release(&mut self, full_heap: bool) {
        self.sweep_large_pages(true);
        debug_assert!(self.treadmill.is_nursery_empty());
        if full_heap {
            self.sweep_large_pages(false);
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
            crate::util::metadata::vo_bit::is_vo_bit_set::<VM>(object),
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
                if nursery_object && self.common.needs_log_bit {
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
            crate::util::metadata::vo_bit::unset_vo_bit::<VM>(object);
            self.pr
                .release_pages(get_super_page(object.to_object_start::<VM>()));
        };
        if sweep_nursery {
            for object in self.treadmill.collect_nursery() {
                sweep(object);
            }
        } else {
            for object in self.treadmill.collect() {
                sweep(object)
            }
        }
    }

    /// Allocate an object
    pub fn allocate_pages(&self, tls: VMThread, pages: usize) -> Address {
        self.acquire(tls, pages)
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
}

fn get_super_page(cell: Address) -> Address {
    cell.align_down(BYTES_IN_PAGE)
}
