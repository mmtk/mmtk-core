use super::space::{CommonSpace, Space, SpaceOptions, SFT};
use crate::util::alloc::allocator;
use crate::util::constants::MIN_OBJECT_SIZE;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::{HeapMeta, MonotonePageResource, PageResource, VMRequest};
use crate::util::metadata::load_metadata;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{compare_exchange_metadata, extract_side_metadata};
use crate::util::{alloc_bit, object_forwarding, Address, ObjectReference};
use crate::{vm::*, AllocationSemantics, CopyContext, TransitiveClosure};
use atomic::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub struct MarkCompactSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    // pr: FreeListPageResource<VM>,
    pr: MonotonePageResource<VM>,
    forwarding_pointers: Mutex<HashMap<Address, Address>>,
    refs: Mutex<HashSet<ObjectReference>>,
}

const GC_MARK_BIT_MASK: usize = 1;

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
        // MarkCompactSpace::<VM>::test_mark_bit(object)
        self.refs.lock().unwrap().contains(&object) && alloc_bit::is_alloced(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        #[cfg(feature = "global_alloc_bit")]
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
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        // self.pr.release_pages(_start);
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
        let local_specs = extract_side_metadata(&[
            *VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
            *VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
            *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        ]);
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
                // FreeListPageResource::new_discontiguous(0, vm_map)
                MonotonePageResource::new_discontiguous(0, vm_map)
            } else {
                // FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
                MonotonePageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
            forwarding_pointers: Mutex::new(HashMap::new()),
            refs: Mutex::new(HashSet::new()),
        }
    }

    pub fn prepare(&self) {}

    pub fn release(&self) {}

    // pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
    //     &self,
    //     trace: &mut T,
    //     object: ObjectReference,
    //     semantics: AllocationSemantics,
    //     copy_context: &mut C,
    // ) -> ObjectReference {
    //     panic!("not implemented since mark compact needs a two phase trace.")
    // }

    pub fn trace_mark_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        trace!("trace_mark_object");
        if MarkCompactSpace::<VM>::test_and_mark(object) {
            trace!("mark is done for the current object");
            #[cfg(feature = "global_alloc_bit")]
            debug_assert!(
                crate::util::alloc_bit::is_alloced(object),
                "{:x}: alloc bit not set",
                object
            );
            trace.process_node(object);
        }
        object
    }

    pub fn trace_forward_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if MarkCompactSpace::<VM>::test_and_clear_mark(object) {
            trace.process_node(object);
        }
        let new_object = *self
            .forwarding_pointers
            .lock()
            .unwrap()
            .get(&object.to_address())
            .unwrap();

        // object
        unsafe { new_object.to_object_reference() }
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

    pub fn test_mark_bit(object: ObjectReference) -> bool {
        load_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        ) == 1
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

    pub fn info(&self) {
        self.compact();
    }

    pub fn calcluate_forwarding_pointer(&self) {
        println!("##############calculate forwarding pointer start##############");
        let mut from = self.common.start;
        let mut to = self.common.start;
        let end = self.pr.cursor();
        println!("end is {}, common.end is {:#x?}", end, self.common.extent);
        while from < end {
            if alloc_bit::is_alloced_object(from) {
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj);

                if Self::to_be_compacted(obj) {
                    to = allocator::align_allocation_no_fill::<VM>(to, 8, 0);
                    self.forwarding_pointers.lock().unwrap().insert(from, to);
                    self.refs
                        .lock()
                        .unwrap()
                        .insert(unsafe { to.to_object_reference() });
                    to += size;
                }
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
        println!("###############calculate forwarding pointer end###############");
    }

    pub fn compact(&self) {
        let mut from = self.common.start;
        let end = self.pr.cursor();
        let mut to = Address::ZERO;
        // println!("##########clear all alloc bit##########");
        // crate::util::alloc_bit::bzero_alloc_bit(
        //     self.common.start,
        //     unsafe { self.pr.get_current_chunk() }
        //         + crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK
        //         - self.common.start,
        // );
        // println!("#######################################");
        while from < end {
            if alloc_bit::is_alloced_object(from) {
                // clear the alloc bit
                alloc_bit::unset_addr_alloc_bit(from);
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj);

                if let Some(target_address) = self.forwarding_pointers.lock().unwrap().get(&from) {
                    to = *target_address;
                    let target = unsafe { target_address.to_object_reference() };
                    // println!("target: {}, size: {}", target, size);
                    // copy obj to target
                    let dst = target.to_address();
                    // Copy
                    let src = obj.to_address();
                    for i in 0..size {
                        unsafe { (dst + i).store((src + i).load::<u8>()) };
                    }
                    // update alloc_bit,
                    alloc_bit::set_alloc_bit(target);
                    to += size
                }

                // VM::VMObjectModel::dump_object(obj);
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
        // reset the bump pointer and clear forwarding_pointers map
        // self.pr.cursor = to
        // release_pages(to)
        // current_chunk = chunk_align_down(to)
        self.pr.reset_cursor(to);
        self.forwarding_pointers.lock().unwrap().clear();
    }
}
