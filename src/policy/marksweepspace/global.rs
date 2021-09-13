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

use super::{
    super::space::{CommonSpace, Space, SpaceOptions, SFT},
    chunks::ChunkMap,
    metadata::{is_alloced, unset_alloc_bit_unsafe},
};
use crate::vm::ObjectModel;

// const NATIVE_MALLOC_SPECS: Vec<SideMetadataSpec> = [
//     SideMetadataSpec {
//         is_global: false,
//         offset:
//         log_num_of_bits: 6,
//         log_min_obj_size: 16,
//     },
// ].to_vec();

pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    // pub marked_blocks: Mutex<HashMap<usize, Vec<free_list_allocator::BlockList>>>,
    /// Allocation status for all chunks in immix space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<MMTkScheduler<VM>>,
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
        scheduler: Arc<MMTkScheduler<VM>>,
    ) -> MarkSweepSpace<VM> {
        let alloc_mark_bits = &mut metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(ALLOC_SIDE_METADATA_SPEC),
            *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        ]);

        let mut local_specs = {
            vec![
                Block::NEXT_BLOCK_TABLE,
                Block::FREE_LIST_TABLE,
                Block::SIZE_TABLE,
                Block::LOCAL_FREE_LIST_TABLE,
                Block::THREAD_FREE_LIST_TABLE,
                Block::TLS_TABLE,
                Block::MARK_TABLE,
                ChunkMap::ALLOC_TABLE,
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
                    local: local_specs,
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
            chunk_map: ChunkMap::new(),
            scheduler,
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
        use crate::util::alloc::free_list_allocator::TRACING_OBJECT;
        if *TRACING_OBJECT.lock().unwrap() == address.as_usize() {
            println!("marking tracing object 0x{:0x}", *TRACING_OBJECT.lock().unwrap());
        }
        if !is_marked::<VM>(object, None) {
            set_mark_bit::<VM>(object, Some(Ordering::SeqCst));
            let block = Block::from(FreeListAllocator::<VM>::get_block(address));
            block.set_state(BlockState::Marked);
            // self.mark_block(block);
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
