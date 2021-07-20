use atomic::Ordering;

use crate::{TransitiveClosure, policy::marksweepspace::metadata::{is_marked, set_mark_bit}, util::{Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, metadata::{MetadataSpec, load_metadata, side_metadata::{SideMetadataContext, SideMetadataSpec}}}, vm::VMBinding};

use super::super::space::{CommonSpace, SFT, Space, SpaceOptions};

pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
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
        if object.is_null() {
            return object;
        }
        let address = object.to_address();
        assert!(
            self.in_space(object),
            "Cannot mark an object {} that was not alloced by free list allocator.",
            address,
        );
        if !is_marked::<VM>(object) {
            set_mark_bit::<VM>(object);
            trace.process_node(object);
        }
        object
    }

    #[inline]
    pub fn get_next_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[0]
    }

    #[inline]
    pub fn get_free_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[1]
    }

    #[inline]
    pub fn get_size_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[2]
    }

    #[inline]
    pub fn get_local_free_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[3]
    }

    #[inline]
    pub fn get_thread_free_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[4]
    }

    #[inline]
    pub fn get_tls_metadata_spec(&self) -> SideMetadataSpec {
        self.common.metadata.local[5]
    }

    pub fn eager_sweep(&self, tls: VMWorkerThread) {
        let mut block = self.common.start;
        while block < self.common.start + self.common.extent {

        }

        unreachable!("start = {}, extent = {}", &self.common.start, &self.common.extent)
    }
    
    pub fn load_block_tls(&self, block: Address) -> OpaquePointer {
        let tls = load_metadata::<VM>(
            MetadataSpec::OnSide(self.get_tls_metadata_spec()), 
            unsafe {block.to_object_reference()},
            None,
            Some(Ordering::SeqCst));
        unsafe {
            std::mem::transmute::<usize, OpaquePointer>(tls)
        }
    }
}