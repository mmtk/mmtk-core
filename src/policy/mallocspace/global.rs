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
use crate::vm::VMBinding;
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
        "MallocSpace"
    }

    fn is_live(&self, _object: ObjectReference) -> bool {
        unimplemented!();
    }
    fn is_movable(&self) -> bool {
        unimplemented!();
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        unimplemented!();
    }
    fn initialize_header(&self, _object: ObjectReference, _alloc: bool) {}
}

impl<VM: VMBinding> Space<VM> for MallocSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        unimplemented!();
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        unimplemented!();
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }
    fn common(&self) -> &CommonSpace<VM> {
        unimplemented!();
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        unimplemented!();
    }

    fn init(&mut self, _vm_map: &'static VMMap) {
        unimplemented!();
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        unimplemented!();
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let address = object.to_address();
        self.address_in_space(address)
    }

    fn address_in_space(&self, start: Address) -> bool {
        meta_space_mapped(start) && load_atomic(ALLOC_METADATA_SPEC, start) == 1
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
            "Cannot mark an object that was not alloced by malloc."
        );
        if !is_marked(address) {
            set_mark_bit(address);
            trace.process_node(object);
        }
        object
    }
}
