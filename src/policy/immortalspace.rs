use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::address::Address;
use crate::util::heap::{MonotonePageResource, PageResource, VMRequest};

use crate::util::constants::CARD_META_PAGES_PER_REGION;
use crate::util::ObjectReference;

use crate::plan::TransitiveClosure;
use crate::util::header_byte::HeaderByte;

use crate::plan::PlanConstraints;
use crate::policy::space::SpaceOptions;
use crate::util::gc_byte;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::vm::VMBinding;

pub struct ImmortalSpace<VM: VMBinding> {
    mark_state: u8,
    common: CommonSpace<VM>,
    pr: MonotonePageResource<VM>,

    header_byte: HeaderByte,
}

const GC_MARK_BIT_MASK: u8 = 1;
const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

impl<VM: VMBinding> SFT for ImmortalSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, _object: ObjectReference) -> bool {
        true
    }
    fn is_movable(&self) -> bool {
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        let old_value = gc_byte::read_gc_byte::<VM>(object);
        let mut new_value = (old_value & GC_MARK_BIT_MASK) | self.mark_state;
        if self.header_byte.needs_unlogged_bit {
            new_value |= self.header_byte.unlogged_bit;
        }
        gc_byte::write_gc_byte::<VM>(object, new_value);
    }
}

impl<VM: VMBinding> Space<VM> for ImmortalSpace<VM> {
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
        constraints: &'static PlanConstraints,
    ) -> Self {
        let common = CommonSpace::new(
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
        );
        ImmortalSpace {
            mark_state: 0,
            pr: if vmrequest.is_discontiguous() {
                MonotonePageResource::new_discontiguous(META_DATA_PAGES_PER_REGION, vm_map)
            } else {
                MonotonePageResource::new_contiguous(
                    common.start,
                    common.extent,
                    META_DATA_PAGES_PER_REGION,
                    vm_map,
                )
            },
            common,
            header_byte: HeaderByte::new(constraints),
        }
    }

    fn test_and_mark(object: ObjectReference, value: u8) -> bool {
        let mut old_value = gc_byte::read_gc_byte::<VM>(object);
        let mut mark_bit = old_value & GC_MARK_BIT_MASK;
        if mark_bit == value {
            return false;
        }
        while !gc_byte::compare_exchange_gc_byte::<VM>(
            object,
            old_value,
            old_value ^ GC_MARK_BIT_MASK,
        ) {
            old_value = gc_byte::read_gc_byte::<VM>(object);
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
