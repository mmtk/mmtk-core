use std::ops::Range;

use super::sft::SFT;
use super::space::{CommonSpace, Space};
use crate::plan::VectorObjectQueue;
use crate::policy::gc_work::{TraceKind, TRACE_KIND_TRANSITIVE_PIN};
use crate::policy::sft::GCWorkerMutRef;
use crate::scheduler::GCWorker;
use crate::util::alloc::allocator::align_allocation_no_fill;
use crate::util::constants::LOG_BYTES_IN_WORD;
use crate::util::copy::CopySemantics;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::metadata::{extract_side_metadata, vo_bit};
use crate::util::{Address, ObjectReference};
use crate::{vm::*, ObjectQueue};
use atomic::Ordering;

pub(crate) const TRACE_KIND_MARK: TraceKind = 0;
pub(crate) const TRACE_KIND_FORWARD: TraceKind = 1;

pub struct MarkCompactSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
}

const GC_MARK_BIT_MASK: u8 = 1;

/// For each MarkCompact object, we need one extra word for storing forwarding pointer (Lisp-2 implementation).
/// Note that considering the object alignment, we may end up allocating/reserving more than one word per object.
/// See [`MarkCompactSpace::HEADER_RESERVED_IN_BYTES`].
pub const GC_EXTRA_HEADER_WORD: usize = 1;
const GC_EXTRA_HEADER_BYTES: usize = GC_EXTRA_HEADER_WORD << LOG_BYTES_IN_WORD;

impl<VM: VMBinding> SFT for MarkCompactSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    fn get_forwarded_object(&self, object: ObjectReference) -> Option<ObjectReference> {
        let forwarding_pointer = Self::get_header_forwarding_pointer(object);
        if forwarding_pointer.is_null() {
            None
        } else {
            Some(forwarding_pointer)
        }
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        // Sanity checker cannot use this method to do the verification
        // since the mark bit will be cleared during the second trace(update forwarding pointer)
        Self::is_marked(object)
    }

    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, _object: ObjectReference) -> bool {
        panic!("Cannot pin/unpin objects of MarkCompactSpace.")
    }

    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        panic!("Cannot pin/unpin objects of MarkCompactSpace.")
    }

    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        false
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        crate::util::metadata::vo_bit::set_vo_bit::<VM>(object);
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr::<VM>(addr).is_some()
    }

    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        _object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        // We should not use trace_object for markcompact space.
        // Depending on which trace it is, we should manually call either trace_mark or trace_forward.
        panic!("sft_trace_object() cannot be used with mark compact space")
    }
}

impl<VM: VMBinding> Space<VM> for MarkCompactSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }

    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        self.common().initialize_sft(self.as_sft(), sft_map)
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("markcompactspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for MarkCompactSpace<VM> {
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        debug_assert!(
            KIND != TRACE_KIND_TRANSITIVE_PIN,
            "MarkCompact does not support transitive pin trace."
        );
        if KIND == TRACE_KIND_MARK {
            self.trace_mark_object(queue, object)
        } else if KIND == TRACE_KIND_FORWARD {
            self.trace_forward_object(queue, object)
        } else {
            unreachable!()
        }
    }
    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        if KIND == TRACE_KIND_MARK {
            false
        } else if KIND == TRACE_KIND_FORWARD {
            true
        } else {
            unreachable!()
        }
    }
}

impl<VM: VMBinding> MarkCompactSpace<VM> {
    /// We need one extra header word for each object. Considering the alignment requirement, this is
    /// the actual bytes we need to reserve for each allocation.
    pub const HEADER_RESERVED_IN_BYTES: usize = if VM::MAX_ALIGNMENT > GC_EXTRA_HEADER_BYTES {
        VM::MAX_ALIGNMENT
    } else {
        GC_EXTRA_HEADER_BYTES
    }
    .next_power_of_two();

    // The following are a few functions for manipulating header forwarding poiner.
    // Basically for each allocation request, we allocate extra bytes of [`HEADER_RESERVED_IN_BYTES`].
    // From the allocation result we get (e.g. `alloc_res`), `alloc_res + HEADER_RESERVED_IN_BYTES` is the cell
    // address we return to the binding. It ensures we have at least one word (`GC_EXTRA_HEADER_WORD`) before
    // the cell address, and ensures the cell address is properly aligned.
    // From the cell address, `cell - GC_EXTRA_HEADER_WORD` is where we store the header forwarding pointer.

    /// Get the address for header forwarding pointer
    fn header_forwarding_pointer_address(object: ObjectReference) -> Address {
        object.to_object_start::<VM>() - GC_EXTRA_HEADER_BYTES
    }

    /// Get header forwarding pointer for an object
    fn get_header_forwarding_pointer(object: ObjectReference) -> ObjectReference {
        unsafe { Self::header_forwarding_pointer_address(object).load::<ObjectReference>() }
    }

    /// Store header forwarding pointer for an object
    fn store_header_forwarding_pointer(
        object: ObjectReference,
        forwarding_pointer: ObjectReference,
    ) {
        unsafe {
            Self::header_forwarding_pointer_address(object)
                .store::<ObjectReference>(forwarding_pointer);
        }
    }

    // Clear header forwarding pointer for an object
    fn clear_header_forwarding_pointer(object: ObjectReference) {
        crate::util::memory::zero(
            Self::header_forwarding_pointer_address(object),
            GC_EXTRA_HEADER_BYTES,
        );
    }

    pub fn new(args: crate::policy::space::PlanCreateSpaceArgs<VM>) -> Self {
        let vm_map = args.vm_map;
        let is_discontiguous = args.vmrequest.is_discontiguous();
        let local_specs = extract_side_metadata(&[*VM::VMObjectModel::LOCAL_MARK_BIT_SPEC]);
        let common = CommonSpace::new(args.into_policy_args(true, false, local_specs));
        MarkCompactSpace {
            pr: if is_discontiguous {
                MonotonePageResource::new_discontiguous(vm_map)
            } else {
                MonotonePageResource::new_contiguous(common.start, common.extent, vm_map)
            },
            common,
        }
    }

    pub fn prepare(&self) {}

    pub fn release(&self) {}

    pub fn trace_mark_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set::<VM>(object),
            "{:x}: VO bit not set",
            object
        );
        if MarkCompactSpace::<VM>::test_and_mark(object) {
            queue.enqueue(object);
        }
        object
    }

    pub fn trace_forward_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set::<VM>(object),
            "{:x}: VO bit not set",
            object
        );
        // from this stage and onwards, mark bit is no longer needed
        // therefore, it can be reused to save one extra bit in metadata
        if MarkCompactSpace::<VM>::test_and_clear_mark(object) {
            queue.enqueue(object);
        }

        Self::get_header_forwarding_pointer(object)
    }

    pub fn test_and_mark(object: ObjectReference) -> bool {
        loop {
            let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
                object,
                None,
                Ordering::SeqCst,
            );
            let mark_bit = old_value & GC_MARK_BIT_MASK;
            if mark_bit != 0 {
                return false;
            }
            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    1,
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

    pub fn test_and_clear_mark(object: ObjectReference) -> bool {
        loop {
            let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
                object,
                None,
                Ordering::SeqCst,
            );
            let mark_bit = old_value & GC_MARK_BIT_MASK;
            if mark_bit == 0 {
                return false;
            }

            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    0,
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

    pub fn is_marked(object: ObjectReference) -> bool {
        let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        let mark_bit = old_value & GC_MARK_BIT_MASK;
        mark_bit != 0
    }

    fn to_be_compacted(object: &ObjectReference) -> bool {
        Self::is_marked(*object)
    }

    /// Linear scan all the live objects in the given memory region
    fn linear_scan_objects(&self, range: Range<Address>) -> impl Iterator<Item = ObjectReference> {
        crate::util::linear_scan::ObjectIterator::<VM, MarkCompactObjectSize<VM>, true>::new(
            range.start,
            range.end,
        )
    }

    pub fn calculate_forwarding_pointer(&self) {
        let mut to_iter = self.pr.iterate_allocated_regions();
        let Some((mut to_cursor, mut to_size)) = to_iter.next() else {
            return;
        };
        let mut to_end = to_cursor + to_size;
        for (from_start, size) in self.pr.iterate_allocated_regions() {
            let from_end = from_start + size;
            // linear scan the contiguous region
            for obj in self
                .linear_scan_objects(from_start..from_end)
                .filter(Self::to_be_compacted)
            {
                let copied_size =
                    VM::VMObjectModel::get_size_when_copied(obj) + Self::HEADER_RESERVED_IN_BYTES;
                let align = VM::VMObjectModel::get_align_when_copied(obj);
                let offset = VM::VMObjectModel::get_align_offset_when_copied(obj);
                // move to_cursor to aliged start address
                to_cursor = align_allocation_no_fill::<VM>(to_cursor, align, offset);
                // move to next to-block if there is no sufficient memory in current region
                if to_cursor + copied_size > to_end {
                    (to_cursor, to_size) = to_iter.next().unwrap();
                    to_end = to_cursor + to_size;
                    to_cursor = align_allocation_no_fill::<VM>(to_cursor, align, offset);
                    debug_assert!(to_cursor + copied_size <= to_end);
                }
                // Get copied object
                let new_obj = VM::VMObjectModel::get_reference_when_copied_to(
                    obj,
                    to_cursor + Self::HEADER_RESERVED_IN_BYTES,
                );
                // update forwarding pointer
                Self::store_header_forwarding_pointer(obj, new_obj);
                trace!(
                    "Calculate forward: {} (size when copied = {}) ~> {} (size = {})",
                    obj,
                    VM::VMObjectModel::get_size_when_copied(obj),
                    to_cursor,
                    copied_size
                );
                // bump to_cursor
                to_cursor += copied_size;
            }
        }
    }

    pub fn compact(&self) {
        let mut to = Address::ZERO;
        for (from_start, size) in self.pr.iterate_allocated_regions() {
            let from_end = from_start + size;
            for obj in self.linear_scan_objects(from_start..from_end) {
                let copied_size = VM::VMObjectModel::get_size_when_copied(obj);
                // clear the VO bit
                vo_bit::unset_vo_bit::<VM>(obj);

                let forwarding_pointer = Self::get_header_forwarding_pointer(obj);

                trace!("Compact {} to {}", obj, forwarding_pointer);
                if !forwarding_pointer.is_null() {
                    let new_object = forwarding_pointer;
                    Self::clear_header_forwarding_pointer(new_object);

                    // copy object
                    trace!(" copy from {} to {}", obj, new_object);
                    let end_of_new_object =
                        VM::VMObjectModel::copy_to(obj, new_object, Address::ZERO);
                    // update VO bit,
                    vo_bit::set_vo_bit::<VM>(new_object);
                    to = new_object.to_object_start::<VM>() + copied_size;
                    debug_assert_eq!(end_of_new_object, to);
                }
            }
        }

        debug!("Compact end: to = {}", to);

        // reset the bump pointer
        self.pr.reset_cursor(to);
    }
}

struct MarkCompactObjectSize<VM>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::util::linear_scan::LinearScanObjectSize for MarkCompactObjectSize<VM> {
    fn size(object: ObjectReference) -> usize {
        VM::VMObjectModel::get_current_size(object)
    }
}
