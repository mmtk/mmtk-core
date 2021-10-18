use super::space::{CommonSpace, Space, SpaceOptions, SFT};
use crate::plan::MARKCOMPACT_CONSTRAINTS;
use crate::util::constants::{BYTES_IN_WORD, MIN_OBJECT_SIZE};
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::{HeapMeta, MonotonePageResource, PageResource, VMRequest};
use crate::util::metadata::load_metadata;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{compare_exchange_metadata, extract_side_metadata};
use crate::util::{alloc_bit, object_forwarding, Address, ObjectReference};
use crate::{vm::*, TransitiveClosure};
use atomic::Ordering;
// use std::collections::{HashMap, HashSet};
// use std::sync::Mutex;

pub struct MarkCompactSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,
    extra_header: usize,
    // forwarding_pointers: Mutex<HashMap<Address, Address>>,
    // refs: Mutex<HashSet<ObjectReference>>,
}

const GC_MARK_BIT_MASK: usize = 1;

const GC_EXTRA_HEADER_SIZE: usize = MARKCOMPACT_CONSTRAINTS.gc_extra_header_words * BYTES_IN_WORD;

impl<VM: VMBinding> SFT for MarkCompactSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    #[inline(always)]
    fn get_forwarded_object(&self, object: ObjectReference) -> Option<ObjectReference> {
        if object_forwarding::is_forwarded::<VM>(object) {
            Some(object_forwarding::read_forwarding_pointer::<VM>(object))
        } else {
            None
        }
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        alloc_bit::is_alloced(object)
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
        self.extra_header = std::cmp::max(
            GC_EXTRA_HEADER_SIZE,
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
            extra_header: 0,
            // forwarding_pointers: Mutex::new(HashMap::new()),
            // refs: Mutex::new(HashSet::new()),
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
        if MarkCompactSpace::<VM>::test_and_clear_mark(object) {
            trace.process_node(object);
        }
        // let new_object = *self
        //     .forwarding_pointers
        //     .lock()
        //     .unwrap()
        //     .get(&object.to_address())
        //     .unwrap();

        // For markcompact, only one word is required to store the forwarding pointer
        // and it is always stored in front of the object start address. However, the
        // number of extra bytes required is determined by the object alignment
        let forwarding_pointer =
            unsafe { (object.to_address() - GC_EXTRA_HEADER_SIZE).load::<Address>() };

        unsafe { (forwarding_pointer + self.extra_header).to_object_reference() }
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

    pub fn to_be_compacted(object: ObjectReference) -> bool {
        let old_value = load_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
        let mark_bit = old_value & GC_MARK_BIT_MASK;
        mark_bit != 0
    }

    pub fn calcluate_forwarding_pointer(&self) {
        let mut from = self.common.start;
        let mut to = self.common.start;
        let end = self.pr.cursor();
        while from < end {
            if alloc_bit::is_alloced_object(from) {
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj) + self.extra_header;

                if Self::to_be_compacted(obj) {
                    let forwarding_pointer_addr = from - GC_EXTRA_HEADER_SIZE;
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
                let size = VM::VMObjectModel::get_current_size(obj) + self.extra_header;

                let forwarding_pointer_addr = from - GC_EXTRA_HEADER_SIZE;
                let forwarding_pointer = unsafe { forwarding_pointer_addr.load::<Address>() };
                if forwarding_pointer != Address::ZERO {
                    to = forwarding_pointer;
                    let object_addr = forwarding_pointer + self.extra_header;
                    // clear forwarding pointer
                    for i in 0..GC_EXTRA_HEADER_SIZE {
                        unsafe {
                            (forwarding_pointer + self.extra_header - GC_EXTRA_HEADER_SIZE + i)
                                .store::<u8>(0);
                            (forwarding_pointer_addr + i).store::<u8>(0);
                        };
                    }
                    let target = unsafe { object_addr.to_object_reference() };
                    // copy obj to target
                    let dst = target.to_address();
                    // Copy
                    let src = obj.to_address();
                    for i in 0..(size - self.extra_header) {
                        unsafe { (dst + i).store((src + i).load::<u8>()) };
                    }
                    // update alloc_bit,
                    alloc_bit::set_alloc_bit(target);
                    to += size
                }
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
        // reset the bump pointer and clear forwarding_pointers map
        self.pr.reset_cursor(to);
    }
}
