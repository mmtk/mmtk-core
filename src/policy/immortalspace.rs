use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource, VMRequest};

use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::ObjectReference;

use crate::plan::TransitiveClosure;
use crate::util::header_byte;
use crate::vm::ObjectModel;

use crate::policy::space::SpaceOptions;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::vm::VMBinding;
// use crate::mmtk::SFT_MAP;
use std::cell::UnsafeCell;

pub struct ImmortalSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM, MonotonePageResource<VM, ImmortalSpace<VM>>>>,
    mark_state: u8,
}

unsafe impl<VM: VMBinding> Sync for ImmortalSpace<VM> {}

const GC_MARK_BIT_MASK: u8 = 1;
const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

impl<VM: VMBinding> SFT for ImmortalSpace<VM> {
    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
    fn is_movable(&self) -> bool {
        false
    }

    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        let old_value = VM::VMObjectModel::read_available_byte(object);
        let mut new_value = (old_value & GC_MARK_BIT_MASK) | self.mark_state;
        if header_byte::NEEDS_UNLOGGED_BIT {
            new_value |= header_byte::UNLOGGED_BIT;
        }
        VM::VMObjectModel::write_available_byte(object, new_value);
    }
    // fn update_sft(&self, start: Address, chunks: usize) -> () {
    //     SFT_MAP.update(self, start, chunks);
    // }
}

impl<VM: VMBinding> Space<VM> for ImmortalSpace<VM> {
    type PR = MonotonePageResource<VM, ImmortalSpace<VM>>;

    fn common(&self) -> &CommonSpace<VM, Self::PR> {
        unsafe { &*self.common.get() }
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM, Self::PR> {
        &mut *self.common.get()
    }

    fn init(&mut self, vm_map: &'static VMMap) {
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };

        let common_mut = self.common_mut();
        if common_mut.vmrequest.is_discontiguous() {
            common_mut.pr = Some(MonotonePageResource::new_discontiguous(
                META_DATA_PAGES_PER_REGION,
                vm_map,
            ));
        } else {
            common_mut.pr = Some(MonotonePageResource::new_contiguous(
                common_mut.start,
                common_mut.extent,
                META_DATA_PAGES_PER_REGION,
                vm_map,
            ));
        }
        common_mut.pr.as_mut().unwrap().bind_space(me);
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immortalspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> ImmortalSpace<VM> {
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        ImmortalSpace {
            common: UnsafeCell::new(CommonSpace::new(
                SpaceOptions {
                    name,
                    movable: false,
                    immortal: true,
                    zeroed,
                    vmrequest,
                },
                vm_map,
                mmapper,
                heap,
            )),
            mark_state: 0,
        }
    }

    fn test_and_mark(object: ObjectReference, value: u8) -> bool {
        let mut old_value = VM::VMObjectModel::prepare_available_bits(object);
        let mut mark_bit = (old_value as u8) & GC_MARK_BIT_MASK;
        if mark_bit == value {
            return false;
        }
        while !VM::VMObjectModel::attempt_available_bits(
            object,
            old_value,
            old_value ^ (GC_MARK_BIT_MASK as usize),
        ) {
            old_value = VM::VMObjectModel::prepare_available_bits(object);
            mark_bit = (old_value as u8) & GC_MARK_BIT_MASK;
            if mark_bit == value {
                return false;
            }
        }
        true
    }

    pub fn prepare(&mut self) {
        self.mark_state = GC_MARK_BIT_MASK - self.mark_state;
    }

    pub fn release(&mut self) {}

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if ImmortalSpace::<VM>::test_and_mark(object, self.mark_state) {
            trace.process_node(object);
        }
        object
    }
}
