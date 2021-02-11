// This struct is not a real space. It exists because certain functions used by malloc-marksweep (plan::poll and allocators::new) require a struct implementing Space and SFT.

use super::space::CommonSpace;
use crate::policy::space::Space;
use crate::policy::space::SFT;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::PageResource;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use std::marker::PhantomData;

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

    fn get_name(&self) -> &'static str {
        "MallocSpace"
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
