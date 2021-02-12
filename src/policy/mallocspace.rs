// This struct is not a real space. It exists because certain functions used by malloc-marksweep (plan::poll and allocators::new) require a struct implementing Space and SFT.

use atomic::Ordering;

use super::space::CommonSpace;
use crate::plan::marksweep::metadata::*;
use crate::policy::space::SFT;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::PageResource;
use crate::util::malloc::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::{
    policy::space::Space,
    util::{
        alloc::malloc_allocator::HEAP_USED, heap::layout::vm_layout_constants::BYTES_IN_CHUNK,
        side_metadata::load_atomic,
    },
};
use std::{collections::HashSet, marker::PhantomData};

pub struct MallocSpace<VM: VMBinding> {
    phantom: PhantomData<VM>,
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
        unimplemented!();
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
                        HEAP_USED.fetch_sub(malloc_usable_size(ptr), Ordering::SeqCst);
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
        ACTIVE_CHUNKS
            .write()
            .unwrap()
            .retain(|c| !released_chunks.contains(&c.as_usize()));
    }
}

impl<VM: VMBinding> MallocSpace<VM> {
    pub fn new() -> Self {
        MallocSpace {
            phantom: PhantomData,
        }
    }
}

impl<VM: VMBinding> Default for MallocSpace<VM> {
    fn default() -> Self {
        Self::new()
    }
}
