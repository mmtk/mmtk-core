// This struct is not a real space. It exists because certain functions (plan::poll and allocators::new) require a struct implementing Space and SFT.
// TODO: find a way to use plan::poll and allocators::new without a dummy struct.

use std::marker::PhantomData;

use crate::{
    policy::space::{Space, SFT},
    util::{
        heap::{layout::heap_layout::VMMap, PageResource},
        Address, ObjectReference,
    },
    vm::VMBinding,
};

use super::space::CommonSpace;

pub struct MallocSpace<VM: VMBinding> {
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> SFT for MallocSpace<VM> {
    fn name(&self) -> &str {
        unimplemented!();
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
}

impl<VM: VMBinding> MallocSpace<VM> {
    pub fn new() -> Self {
        MallocSpace {
            phantom: PhantomData,
        }
    }
}
