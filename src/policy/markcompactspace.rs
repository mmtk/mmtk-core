use super::space::{CommonSpace, Space, SpaceOptions, SFT};
use crate::util::alloc::allocator::align_allocation_no_fill;
use crate::util::constants::{LOG_BYTES_IN_WORD, MIN_OBJECT_SIZE};
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::{HeapMeta, MonotonePageResource, PageResource, VMRequest};
use crate::util::metadata::load_metadata;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{compare_exchange_metadata, extract_side_metadata};
use crate::util::{alloc_bit, Address, ObjectReference};
use crate::{vm::*, TransitiveClosure};
use atomic::Ordering;

pub struct MarkCompactSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
    header_reserved_in_bytes: usize,
}

const GC_MARK_BIT_MASK: usize = 1;

pub const GC_EXTRA_HEADER_WORD: usize = 1;
const GC_EXTRA_HEADER_BYTES: usize = GC_EXTRA_HEADER_WORD << LOG_BYTES_IN_WORD;

impl<VM: VMBinding> SFT for MarkCompactSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    #[inline(always)]
    fn get_forwarded_object(&self, _object: ObjectReference) -> Option<ObjectReference> {
        // the current forwarding pointer implementation will store forwarding pointers
        // in dead objects' header, which is not the case in mark compact. Mark compact
        // implements its own forwarding mechanism
        unimplemented!()
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        // Sanity checker cannot use this method to do the verification
        // since the mark bit will be cleared during the second trace(update forwarding pointer)
        Self::is_marked(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        crate::util::alloc_bit::set_alloc_bit(object);
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
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

    fn init(&mut self, _vm_map: &'static VMMap) {
        self.common().init(self.as_space());
        self.header_reserved_in_bytes = std::cmp::max(
            GC_EXTRA_HEADER_BYTES,
            VM::VMObjectModel::object_alignment() as usize,
        )
        .next_power_of_two();
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("markcompactspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> MarkCompactSpace<VM> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let local_specs = extract_side_metadata(&[*VM::VMObjectModel::LOCAL_MARK_BIT_SPEC]);
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                needs_log_bit: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: local_specs,
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        MarkCompactSpace {
            pr: if vmrequest.is_discontiguous() {
                MonotonePageResource::new_discontiguous(0, vm_map)
            } else {
                MonotonePageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
            header_reserved_in_bytes: 0,
        }
    }

    pub fn prepare(&self) {}

    pub fn release(&self) {}

    pub fn trace_mark_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(
            crate::util::alloc_bit::is_alloced(object),
            "{:x}: alloc bit not set",
            object
        );
        if MarkCompactSpace::<VM>::test_and_mark(object) {
            trace.process_node(object);
        }
        object
    }

    pub fn trace_forward_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(
            crate::util::alloc_bit::is_alloced(object),
            "{:x}: alloc bit not set",
            object
        );
        // from this stage and onwards, mark bit is no longer needed
        // therefore, it can be reused to save one extra bit in metadata
        if MarkCompactSpace::<VM>::test_and_clear_mark(object) {
            trace.process_node(object);
        }

        // For markcompact, only one word is required to store the forwarding pointer
        // and it is always stored in front of the object start address. However, the
        // number of extra bytes required is determined by the object alignment
        let forwarding_pointer =
            unsafe { (object.to_address() - GC_EXTRA_HEADER_BYTES).load::<Address>() };

        unsafe { (forwarding_pointer + self.header_reserved_in_bytes).to_object_reference() }
    }

    pub fn test_and_mark(object: ObjectReference) -> bool {
        loop {
            let old_value = load_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                None,
                Some(Ordering::SeqCst),
            );
            let mark_bit = old_value & GC_MARK_BIT_MASK;
            if mark_bit != 0 {
                return false;
            }
            if compare_exchange_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                old_value,
                1,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                break;
            }
        }
        true
    }

    pub fn test_and_clear_mark(object: ObjectReference) -> bool {
        loop {
            let old_value = load_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                None,
                Some(Ordering::SeqCst),
            );
            let mark_bit = old_value & GC_MARK_BIT_MASK;
            if mark_bit == 0 {
                return false;
            }

            if compare_exchange_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                old_value,
                0,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                break;
            }
        }
        true
    }

    pub fn is_marked(object: ObjectReference) -> bool {
        let old_value = load_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
        let mark_bit = old_value & GC_MARK_BIT_MASK;
        mark_bit != 0
    }

    pub fn to_be_compacted(object: ObjectReference) -> bool {
        Self::is_marked(object)
    }

    pub fn calculate_forwarding_pointer(&self) {
        let mut from = self.common.start;
        let mut to = self.common.start;
        let end = self.pr.cursor();
        while from < end {
            if alloc_bit::is_alloced_object(from) {
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj) + self.header_reserved_in_bytes;

                if Self::to_be_compacted(obj) {
                    let size = VM::VMObjectModel::get_size_when_copied(obj)
                        + self.header_reserved_in_bytes;
                    let align = VM::VMObjectModel::get_align_when_copied(obj);
                    let offset = VM::VMObjectModel::get_align_offset_when_copied(obj);
                    to = align_allocation_no_fill::<VM>(to, align, offset);
                    let forwarding_pointer_addr = from - GC_EXTRA_HEADER_BYTES;
                    unsafe { forwarding_pointer_addr.store(to) }
                    to += size;
                }
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
    }

    pub fn compact(&self) {
        let mut from = self.common.start;
        let end = self.pr.cursor();
        let mut to = end;
        while from < end {
            if alloc_bit::is_alloced_object(from) {
                // clear the alloc bit
                alloc_bit::unset_addr_alloc_bit(from);
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj) + self.header_reserved_in_bytes;

                let forwarding_pointer_addr = from - GC_EXTRA_HEADER_BYTES;
                let forwarding_pointer = unsafe { forwarding_pointer_addr.load::<Address>() };
                if forwarding_pointer != Address::ZERO {
                    to = forwarding_pointer;
                    let object_addr = forwarding_pointer + self.header_reserved_in_bytes;
                    // clear forwarding pointer
                    crate::util::memory::zero(
                        forwarding_pointer + self.header_reserved_in_bytes - GC_EXTRA_HEADER_BYTES,
                        GC_EXTRA_HEADER_BYTES,
                    );
                    crate::util::memory::zero(forwarding_pointer_addr, GC_EXTRA_HEADER_BYTES);
                    // copy object
                    let target = unsafe { object_addr.to_object_reference() };
                    VM::VMObjectModel::copy_to(obj, target, Address::ZERO);
                    // update alloc_bit,
                    alloc_bit::set_alloc_bit(target);
                    to += size
                }
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
        // reset the bump pointer
        self.pr.reset_cursor(to);
    }
}
