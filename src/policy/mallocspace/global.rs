use super::metadata::*;
use crate::plan::TransitiveClosure;
use crate::policy::space::CommonSpace;
use crate::policy::space::SFT;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::PageResource;
use crate::util::heap::{layout::vm_layout_constants::PAGES_IN_CHUNK, MonotonePageResource};
use crate::util::malloc::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::conversions;
use crate::vm::VMBinding;
use crate::vm::{ActivePlan, ObjectModel};
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::mmapper::Mmapper as IMmapper;
use crate::{
    policy::space::Space,
    util::{heap::layout::vm_layout_constants::BYTES_IN_CHUNK, side_metadata::load_atomic},
};
use std::{collections::HashSet, marker::PhantomData};

const META_DATA_PAGES_PER_REGION: usize = crate::util::constants::CARD_META_PAGES_PER_REGION;
pub struct MallocSpace<VM: VMBinding> {
    phantom: PhantomData<VM>,
    pr: MonotonePageResource<VM>,
}

impl<VM: VMBinding> SFT for MallocSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }

    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!();
    }
    fn is_movable(&self) -> bool {
        false
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        set_alloc_bit(object.to_address());
    }
}

impl<VM: VMBinding> Space<VM> for MallocSpace<VM> {
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
        unreachable!()
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        unreachable!()
    }

    fn init(&mut self, _vm_map: &'static VMMap) {

    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unreachable!()
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let address = object.to_address();
        self.address_in_space(address)
    }

    fn address_in_space(&self, start: Address) -> bool {
        is_meta_space_mapped(start) && load_atomic(ALLOC_METADATA_SPEC, start) == 1
    }

    fn get_name(&self) -> &'static str {
        "MallocSpace"
    }

    fn reserved_pages(&self) -> usize {
        self.pr.common().get_reserved()
    }

    unsafe fn release_all_chunks(&self) {
        let mut released_chunks = HashSet::new();
        for chunk_start in &*ACTIVE_CHUNKS.read().unwrap() {
            let mut chunk_is_empty = true;
            let mut address = *chunk_start;
            let chunk_end = chunk_start.add(BYTES_IN_CHUNK);
            while address.as_usize() < chunk_end.as_usize() {
                if load_atomic(ALLOC_METADATA_SPEC, address) == 1 {
                    if !is_marked(address) {
                        let ptr = address.to_mut_ptr();
                        free(ptr);
                        unset_alloc_bit(address);
                    } else {
                        unset_mark_bit(address);
                        chunk_is_empty = false;
                    }
                }
                address = address.add(VM::MAX_ALIGNMENT);
            }
            if chunk_is_empty {
                released_chunks.insert(chunk_start.as_usize());
            }
        }
        self.pr
            .common()
            .release_reserved(PAGES_IN_CHUNK * released_chunks.len());

        ACTIVE_CHUNKS
            .write()
            .unwrap()
            .retain(|c| !released_chunks.contains(&c.as_usize()));
    }
}

impl<VM: VMBinding> MallocSpace<VM> {
    pub fn new(vm_map: &'static VMMap) -> Self {
        MallocSpace {
            phantom: PhantomData,
            pr: MonotonePageResource::new_discontiguous(META_DATA_PAGES_PER_REGION, vm_map),
        }
    }

    pub fn alloc(&self, size: usize) -> Address {
        let address = Address::from_mut_ptr(unsafe { calloc(1, size) });
        if !address.is_zero() {
            if !is_meta_space_mapped(address) {
                VM::VMActivePlan::global().poll(false, self);
                let chunk_start = conversions::chunk_align_down(address);
                map_meta_space_for_chunk(chunk_start);
                self.get_page_resource()
                    .reserve_pages(PAGES_IN_CHUNK);
            }
        }
        address
    }

    #[inline]
    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let address = object.to_address();
        assert!(
            self.address_in_space(address),
            "Cannot mark an object {} that was not alloced by malloc.",
            address,
        );
        if !is_marked(address) {
            set_mark_bit(address);
            trace.process_node(object);
        }
        object
    }
}
