use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use super::space::{CommonSpace, Space, SpaceOptions, SFT};

use crate::util::alloc::allocator;
use crate::util::constants::MIN_OBJECT_SIZE;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::layout::vm_layout_constants::HEAP_START;
use crate::util::heap::{HeapMeta, MonotonePageResource, PageResource, VMRequest};
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{compare_exchange_metadata, extract_side_metadata};
use crate::util::metadata::{load_metadata, store_metadata};
use crate::util::{alloc_bit, object_forwarding, Address, ObjectReference};
use crate::{vm::*, AllocationSemantics, CopyContext, TransitiveClosure};
use atomic::Ordering;
use atomic_traits::Atomic;

pub struct MarkCompactSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    // pr: FreeListPageResource<VM>,
    pr: MonotonePageResource<VM>,
    mark_counter: AtomicUsize,
    refs: Mutex<HashSet<ObjectReference>>,
    refs_linear: Mutex<HashSet<ObjectReference>>,
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
        MarkCompactSpace::<VM>::test_mark_bit(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: ObjectReference, alloc: bool) {
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
            mark_counter: AtomicUsize::new(0),
            refs: Mutex::new(HashSet::new()),
            refs_linear: Mutex::new(HashSet::new()),
        }
    }

    pub fn prepare(&self) {}

    pub fn release(&self) {}

    pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        semantics: AllocationSemantics,
        copy_context: &mut C,
    ) -> ObjectReference {
        panic!("not implemented since mark compact needs a two phase trace.")
    }

    pub fn trace_mark_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        trace!("trace_mark_object");
        if MarkCompactSpace::<VM>::test_and_mark(object) {
            trace!("mark is done for the current object");
            self.mark_counter.fetch_add(1, Ordering::SeqCst);
            self.refs.lock().unwrap().insert(object);
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
            self.mark_counter.fetch_sub(1, Ordering::SeqCst);
        }
        // let new_object = object_forwarding::read_forwarding_pointer::<VM>(object);
        // trace!("forwarding {}", object);
        // trace!(" -> {}", new_object);
        // debug_assert!(!new_object.is_null(), "forwarding pointer cannot be null");
        //object_forwarding::read_forwarding_pointer::<VM>(object)
        object
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
        mark_bit != 0 && !object_forwarding::is_forwarded::<VM>(object)
    }

    pub fn info(&self) {
        println!(
            "mark_counter: {} ",
            self.mark_counter.load(Ordering::SeqCst)
        );
    }

    pub fn calcluate_forwarding_pointer(&self) {
        let mut from = self.common.start;
        let mut to = self.common.start;
        let end = self.pr.cursor();

        // let end = unsafe { Address::from_usize(0x20000af0000) };
        println!("end is {}, common.end is {}", end, self.common.extent);
        let mut counter = 0;
        while from < end {
            // assert!(from >= to, "from and to mismatched");
            // assert!((from & (0x7 as usize)) == 0, "alignment not satisified");
            if alloc_bit::is_alloced_object(from) {
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj);
                if Self::to_be_compacted(obj) {
                    // to = allocator::align_allocation_no_fill::<VM>(to, 8, 0);
                    // assert!((to & (0x7 as usize)) == 0, "alignment not satisified");
                    // object_forwarding::write_forwarding_pointer::<VM>(obj, unsafe {
                    //     to.to_object_reference()
                    // });
                    // store_metadata::<VM>(
                    //     &VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
                    //     obj,
                    //     3 as usize, // FORWARDED
                    //     None,
                    //     Some(Ordering::SeqCst),
                    // );
                    counter += 1;
                    to += size;
                    self.refs_linear.lock().unwrap().insert(obj);
                }

                // VM::VMObjectModel::dump_object(obj);
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
        println!("############################");
        let mut i = 0 as usize;
        let v1 = self.refs.lock().unwrap().to_owned();
        let v2 = self.refs_linear.lock().unwrap().to_owned();
        let mut m: usize = 0;
        for v in &v1 {
            if !v2.contains(v) {
                if v.value() > m {
                    m = v.value()
                }
                assert!(v.to_address() <= end, "heap boundary is incorrect");
            }
        }
        println!("highest address: {:#04x?}", m);
        println!("############################");
        println!("counter: {}", counter);
    }

    pub fn compact(&self) {
        let start = HEAP_START;
        let mut from = start;
        let mut to = start;
        let end = self.pr.cursor();
        while from < end {
            assert!(from >= to, "from and to mismatched");
            assert!((from & (0x7 as usize)) == 0, "alignment not satisified");
            if alloc_bit::is_alloced_object(from) {
                let obj = unsafe { from.to_object_reference() };
                let size = VM::VMObjectModel::get_current_size(obj);
                if object_forwarding::is_forwarded::<VM>(obj) {
                    alloc_bit::unset_alloc_bit(obj);
                    let target = object_forwarding::read_forwarding_pointer::<VM>(obj);
                    // copy obj to target
                    let dst = target.to_address();
                    // Copy
                    let src = obj.to_address();
                    for i in 0..size {
                        unsafe { (dst + i).store((src + i).load::<u8>()) };
                    }
                    // clear the forwarding bit, update alloc_bit,
                    object_forwarding::clear_forwarding_bits::<VM>(obj);
                    alloc_bit::unset_alloc_bit(obj);
                    alloc_bit::set_alloc_bit(target);
                    to = target.to_address();
                }

                // VM::VMObjectModel::dump_object(obj);
                from += size;
            } else {
                from += MIN_OBJECT_SIZE
            }
        }
        // reset the bump pointer
    }
}
