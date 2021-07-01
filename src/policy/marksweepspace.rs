use crate::{TransitiveClosure, util::{ObjectReference, constants::CARD_META_PAGES_PER_REGION, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, side_metadata::{SideMetadataContext, SideMetadataSpec}}, vm::VMBinding};

use super::space::{CommonSpace, SFT, Space, SpaceOptions};
const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

pub struct MarkSweepSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>    
}

impl<VM: VMBinding> SFT for MarkSweepSpace<VM> {
    fn name(&self) -> &str {
        self.common.name
    }

    fn is_live(&self, object: crate::util::ObjectReference) -> bool {
        todo!()
    }

    fn is_movable(&self) -> bool {
        todo!()
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        todo!()
    }

    fn initialize_object_metadata(&self, object: crate::util::ObjectReference, alloc: bool) {
        // todo!()
        // do nothing for now
    }
}

impl<VM: VMBinding> Space<VM> for MarkSweepSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }

    fn get_page_resource(&self) -> &dyn crate::util::heap::PageResource<VM> {
        &self.pr
    }

    fn init(&mut self, vm_map: &'static crate::util::heap::layout::heap_layout::VMMap) {
        self.common().init(self.as_space());
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, start: crate::util::Address) {
        todo!()
    }
}

impl<VM: VMBinding> MarkSweepSpace<VM> {
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        local_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> MarkSweepSpace<VM> {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: vec![],
                    local: local_side_metadata_specs
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        MarkSweepSpace {
            pr: if vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
        }
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        todo!()
    }

    fn mark_object(
        &self,
        object: ObjectReference,
    ) {
        todo!()
    }
}