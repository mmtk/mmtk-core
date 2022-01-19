use std::{
    sync::{Arc},
};

use atomic::Ordering;

use crate::{TransitiveClosure, policy::{marksweepspace::{block::{Block, BlockState}, chunk::Chunk, metadata::{is_marked, set_mark_bit}}, space::SpaceOptions, mallocspace::metadata::is_alloced}, scheduler::{GCWorkScheduler, WorkBucketStage}, util::{ ObjectReference, alloc_bit::{ALLOC_SIDE_METADATA_SPEC, bzero_alloc_bit}, heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        }, metadata::{self, MetadataSpec, side_metadata::{self, SideMetadataContext, SideMetadataSpec}, store_metadata}, alloc::free_list_allocator::mi_bin}, vm::VMBinding, memory_manager::is_live_object};

use super::{super::space::{CommonSpace, Space, SFT}, chunk::{ChunkMap, ChunkState}};
use crate::vm::ObjectModel;
use crate::util::alloc::free_list_allocator::{BlockLists, BLOCK_LISTS_EMPTY};
use std::sync::Mutex;
use crate::util::Address;
use crate::util::VMThread;
use crate::util::constants::LOG_BYTES_IN_PAGE;

pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    /// Allocation status for all chunks in MS space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
    pub abandoned: Mutex<BlockLists>,
}

impl<VM: VMBinding> SFT for MarkSweepSpace<VM> {
    fn name(&self) -> &str {
        "MarkSweepSpace"
    }

    fn is_live(&self, object: crate::util::ObjectReference) -> bool {
        true
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, object: crate::util::ObjectReference, alloc: bool) {
        // do nothing
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
    /// Get work packet scheduler
    fn scheduler(&self) -> &GCWorkScheduler<VM> {
        &self.scheduler
    }

    pub fn new(
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
            abandoned: Mutex::from(BLOCK_LISTS_EMPTY),
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
            let block = Block::from(Block::align(address));
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
        self.zero_mark_bits();
    }

    pub fn block_level_sweep(&self) {
        let space = unsafe { &*(self as *const Self) };
        // for chunk in self.chunk_map.all_chunks() {
        //     chunk.sweep(space);
        // }
        let work_packets = self.chunk_map.generate_sweep_tasks(space);
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
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
        bzero_alloc_bit(block.start(), Block::BYTES);
    }

    pub fn acquire_block(&self, tls: VMThread, size: usize) -> (Address, bool) {
        // returns true if block is abandoned and recycled, else false
        //later, change this to return block and init metadata here
        let bin = mi_bin(size);
        let mut abandoned = self.abandoned.lock().unwrap();
        if (abandoned)[bin].is_empty() {
            (self.acquire(tls, Block::BYTES >> LOG_BYTES_IN_PAGE), false)
        } else {
            (abandoned[bin].pop::<VM>().start(), true)
        }

    }
}