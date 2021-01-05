use std::marker::PhantomData;

use crate::{policy::space::{Space, SFT}, util::{Address, ObjectReference, heap::{PageResource, layout::heap_layout::VMMap}}, vm::VMBinding};

use super::space::CommonSpace;

pub struct MSSpace<VM: VMBinding> {
    phantom: PhantomData<VM>
}

impl<VM: VMBinding> SFT for MSSpace<VM> {
    fn name(&self) -> &str {
        "MSSpace"
    }

    fn is_live(&self, object: ObjectReference) -> bool {
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

impl<VM: VMBinding> Space<VM> for MSSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
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

impl<VM: VMBinding> MSSpace<VM> {
    pub fn new() -> Self {
        MSSpace {
            phantom: PhantomData,
        }
    }

}