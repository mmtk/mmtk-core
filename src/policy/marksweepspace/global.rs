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

use atomic::Ordering;

use crate::{TransitiveClosure, policy::{marksweepspace::{block::{Block, BlockState}, chunk::Chunk, metadata::{is_marked, set_mark_bit}}, space::SpaceOptions, mallocspace::metadata::is_alloced}, scheduler::{GCWorkScheduler, WorkBucketStage}, util::{ ObjectReference, alloc_bit::{ALLOC_SIDE_METADATA_SPEC, bzero_alloc_bit}, heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        }, metadata::{self, MetadataSpec, side_metadata::{self, SideMetadataContext, SideMetadataSpec}, store_metadata}, alloc::free_list_allocator::mi_bin}, vm::VMBinding, memory_manager::is_live_object};

use super::{super::space::{CommonSpace, Space, SFT}, chunks::{ChunkMap, ChunkState}};
use crate::vm::ObjectModel;
use crate::util::alloc::free_list_allocator::{BlockLists, BLOCK_LISTS_EMPTY};
use std::sync::Mutex;
use crate::util::Address;
use crate::util::VMThread;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::alloc::free_list_allocator::MI_BIN_FULL;

pub enum BlockAcquireResult {
    Fresh(Block),
    AbandonedAvailable(Block),
    AbandondedUnswept(Block),
}

pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    /// Allocation status for all chunks in MS space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
    pub abandoned_available: Mutex<BlockLists>,
    pub abandoned_unswept: Mutex<BlockLists>,
    pub abandoned_consumed: Mutex<BlockLists>,
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
        true
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
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> MarkSweepSpace<VM> {
        let alloc_bits = &mut metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(ALLOC_SIDE_METADATA_SPEC),
        ]);

        let mark_bits = &mut metadata::extract_side_metadata(&[
            *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
        ]);

        let mut local_specs = {
            metadata::extract_side_metadata(
            &vec![
                MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
                MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
                MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::SIZE_TABLE),
                MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::BLOCK_LIST_TABLE),
                MetadataSpec::OnSide(Block::TLS_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            ]
            )
        };

        local_specs.append(mark_bits);

        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                needs_log_bit: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: alloc_bits.to_vec(),
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
            abandoned_available: Mutex::from(BLOCK_LISTS_EMPTY),
            abandoned_unswept: Mutex::from(BLOCK_LISTS_EMPTY),
            abandoned_consumed: Mutex::from(BLOCK_LISTS_EMPTY),
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
        if !is_marked::<VM>(object, Some(Ordering::SeqCst)) {
            set_mark_bit::<VM>(object, Some(Ordering::SeqCst));
            let block = FreeListAllocator::<VM>::get_block(address);
            block.set_state(BlockState::Marked);
            trace.process_node(object);
        }
        object
    }
        
    pub fn zero_mark_bits(&self) {
        // todo: concurrent zeroing
        use crate::vm::*;
        for chunk in self.chunk_map.all_chunks() {
            if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
                side_metadata::bzero_metadata(&side, chunk.start(), Chunk::BYTES);
            }
        }
    }

    pub fn check_valid_blocklists(&self) {
        use crate::util::alloc::free_list_allocator::MI_LARGE_OBJ_WSIZE_MAX;
        use crate::vm::*;
        for chunk in self.chunk_map.all_chunks() {
            for block in chunk.blocks() {
                if block.get_state() != BlockState::Unallocated {
                    let block_list = block.load_block_list::<VM>();
                    if !block_list.is_null() {
                        unsafe {
                            assert!((*block_list).size == block.load_block_cell_size::<VM>());
                        }
                    }
                }
            }
        }
    }

    pub fn block_has_no_objects(&self, block: Block) -> bool {
        // for debugging, delete this later
        // assumes block is allocated (has metadata)
        let size = block.load_block_cell_size::<VM>();
        let mut cell = block.start();
        while cell < block.start() + Block::BYTES {
            if is_alloced(unsafe { cell.to_object_reference() }) {
                return false;
            }
            cell += size;
        }
        return true;
    }

    pub fn record_new_block(&self, block: Block) {
        block.init();
        self.chunk_map.set(block.chunk(), ChunkState::Allocated);
    }

    #[inline]
    pub fn get_next_metadata_spec(&self) -> SideMetadataSpec {
        Block::NEXT_BLOCK_TABLE
    }

    pub fn reset(&mut self) {
        // do nothing
        self.zero_mark_bits();
    }

    pub fn block_level_sweep(&self) {
        let space = unsafe { &*(self as *const Self) };
        let work_packets = self.chunk_map.generate_sweep_tasks(space);
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
        let mut abandoned_unswept = self.abandoned_unswept.lock().unwrap();
        let mut abandoned_consumed = self.abandoned_consumed.lock().unwrap();
        let mut i = 0;
        while i < MI_BIN_FULL {
            if !abandoned_consumed[i].is_empty() {
                abandoned_consumed[i].lock();
                abandoned_unswept[i].lock();
                abandoned_unswept[i].append::<VM>(&mut abandoned_consumed[i]);
                abandoned_unswept[i].unlock();
                abandoned_consumed[i].unlock();
            }
            i += 1;
        }
    }

    /// Release a block.
    pub fn release_block(&self, block: Block) {
        self.block_clear_metadata(block);

        block.deinit();
        self.pr.release_pages(block.start());
    }

    pub fn block_clear_metadata(&self, block: Block) {
        for metadata_spec in &self.common.metadata.local {
            store_metadata::<VM>(
                &MetadataSpec::OnSide(*metadata_spec),
                unsafe { block.start().to_object_reference() },
                0,
                None,
                Some(Ordering::SeqCst),
            )
        }
        bzero_alloc_bit(block.start(), BYTES_IN_BLOCK);
    }

    pub fn acquire_block(&self, tls: VMThread, size: usize) -> BlockAcquireResult {
        let bin = mi_bin(size);

        {
            let mut abandoned = self.abandoned_available.lock().unwrap();
            if !abandoned[bin].is_empty() {
                let block = Block::from(abandoned[bin].pop::<VM>().start());
                return BlockAcquireResult::AbandonedAvailable(block);
            }
        }

        {
            let mut abandoned_unswept = self.abandoned_unswept.lock().unwrap();
            if !abandoned_unswept[bin].is_empty() {
                let block = Block::from(abandoned_unswept[bin].pop::<VM>().start());
                return BlockAcquireResult::AbandondedUnswept(block);
            }
        }
        BlockAcquireResult::Fresh(Block::from(self.acquire(tls, Block::BYTES >> LOG_BYTES_IN_PAGE)))
    }
}
