use crate::{TransitiveClosure, util::{Address, ObjectReference, constants::CARD_META_PAGES_PER_REGION, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, side_metadata::{SideMetadataContext, SideMetadataSpec}}, vm::VMBinding};

use crate::{
    policy::marksweepspace::{
        block::{Block, BlockState},
        metadata::{is_marked, set_mark_bit, unset_mark_bit, ALLOC_SIDE_METADATA_SPEC},
    },
    scheduler::{MMTkScheduler, WorkBucketStage},
    util::{
        alloc::free_list_allocator::{self, FreeListAllocator, BLOCK_LISTS_EMPTY, BYTES_IN_BLOCK},
        constants::LOG_BYTES_IN_PAGE,
        heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        },
        metadata::{
            self, compare_exchange_metadata, load_metadata,
            side_metadata::{
                SideMetadataContext, SideMetadataOffset, SideMetadataSpec,
                LOCAL_SIDE_METADATA_BASE_ADDRESS,
            },
            store_metadata, MetadataSpec,
        },
        Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread,
    },
    vm::VMBinding,
    TransitiveClosure,
};

use crate::{TransitiveClosure, policy::marksweepspace::metadata::{ALLOC_SIDE_METADATA_SPEC, is_marked, set_mark_bit, unset_mark_bit}, util::{Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread, alloc::free_list_allocator::{self, BYTES_IN_BLOCK, FreeListAllocator}, heap::{FreeListPageResource, HeapMeta, VMRequest, layout::heap_layout::{Mmapper, VMMap}}, metadata::{self, MetadataSpec, compare_exchange_metadata, load_metadata, side_metadata::{LOCAL_SIDE_METADATA_BASE_ADDRESS, SideMetadataContext, SideMetadataSpec, metadata_address_range_size}, store_metadata}}, vm::VMBinding};

use super::{super::space::{CommonSpace, SFT, Space, SpaceOptions}, metadata::{is_alloced, unset_alloc_bit}};
use crate::vm::ObjectModel;

pub struct MarkSweepSpace<VM: VMBinding> {
    pub active_blocks: Mutex<HashSet<Address>>,
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    marked_blocks: HashMap<usize, Vec<free_list_allocator::BlockQueue>>
}

impl<VM: VMBinding> SFT for MarkSweepSpace<VM> {
    fn name(&self) -> &str {
        todo!()
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
        todo!()
    }
}

impl<VM: VMBinding> Space<VM> for MarkSweepSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        todo!()
    }

    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        todo!()
    }

    fn get_page_resource(&self) -> &dyn crate::util::heap::PageResource<VM> {
        &self.pr
    }

    fn init(&mut self, vm_map: &'static crate::util::heap::layout::heap_layout::VMMap) {
        todo!()
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, start: crate::util::Address) {
        todo!()
    }
}

impl<VM: VMBinding> MarkSweepSpace<VM> {
    fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        // local_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> MarkSweepSpace<VM> {
        let alloc_mark_bits = &mut metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(ALLOC_SIDE_METADATA_SPEC),
            VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        ]);
        let side_metadata_next = SideMetadataSpec {
            is_global: false,
            offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize() + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_size = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_local_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_thread_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_tls = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&side_metadata_thread_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };

        let side_metadata_marked = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&side_metadata_thread_free) + metadata_address_range_size(&alloc_mark_bits[0]) + metadata_address_range_size(&alloc_mark_bits[0]),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let mut local_specs = {
            vec![
                side_metadata_next,
                side_metadata_free,
                side_metadata_size,
                side_metadata_local_free,
                side_metadata_thread_free,
                side_metadata_tls,
                side_metadata_marked,
            ]
        };

        local_specs.append(alloc_mark_bits);

        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: vec![],
                    local: local_specs
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        MarkSweepSpace {
            active_blocks: Mutex::default(),
            pr: if vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
            marked_blocks: HashMap::default(),
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
            let block = FreeListAllocator::<VM>::get_block(address);
            self.mark_block(block);
            trace.process_node(object);
        }
        object
    }

    pub fn acquire_block(&self) -> Address {
        // acquire 64kB block from the global pool
        todo!()
    }

    pub fn return_block(&self) {
        // return freed 64kB block
        todo!()
    }

}
