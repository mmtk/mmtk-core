use std::{
    sync::{Arc},
};

use atomic::Ordering;

use crate::{TransitiveClosure, policy::{marksweepspace::{block::{Block, BlockState}, chunks::Chunk, metadata::{is_marked, set_mark_bit}}, space::SpaceOptions}, scheduler::{GCWorkScheduler, WorkBucketStage}, util::{Address, ObjectReference, OpaquePointer, VMThread, VMWorkerThread, alloc::free_list_allocator::{self, FreeListAllocator, BLOCK_LISTS_EMPTY, BYTES_IN_BLOCK}, alloc_bit::{ALLOC_SIDE_METADATA_SPEC, bzero_alloc_bit}, constants::LOG_BYTES_IN_PAGE, heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        }, metadata::{self, MetadataSpec, load_metadata, side_metadata::{self, SideMetadataContext, SideMetadataSpec, address_to_meta_address}, store_metadata}}, vm::VMBinding};

use super::{super::space::{CommonSpace, Space, SFT}, chunks::{ChunkMap, ChunkState}};
use crate::vm::ObjectModel;

pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    /// Allocation status for all chunks in MS space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
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
                MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::SIZE_TABLE),
                MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
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
        // eprintln!("check mark {}", object.to_address());
        if !is_marked::<VM>(object, Some(Ordering::SeqCst)) {
            set_mark_bit::<VM>(object, Some(Ordering::SeqCst));
            // eprintln!("m {} meta: {}", object.to_address(), address_to_meta_address(&VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.extract_side_spec(), object.to_address()));
            let block = Block::from(FreeListAllocator::<VM>::get_block(address));
            block.set_state(BlockState::Marked);
            trace.process_node(object);
        }
        object
    }
        
    pub fn zero_mark_bits(&self) {
        // eprintln!("b zero all {} {}", self.chunk_map.all_chunks().start.start(), self.chunk_map.all_chunks().end.start());
        use crate::vm::*;
        for chunk in self.chunk_map.all_chunks() {
            // eprintln!("b zero {}", chunk.start());
            if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
                side_metadata::bzero_metadata(&side, chunk.start(), Chunk::BYTES);
            }
        }
    }

    pub fn record_new_block(&self, block: Address) {
        let block = Block::from(block);
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
    }

    /// Release a block.
    pub fn release_block(&self, block: Address) {
        // eprintln!("b < 0x{:0x}", block);
        self.block_clear_metadata(block);

        let block = Block::from(block);
        block.deinit();
        self.pr.release_pages(block.start());
    }

    pub fn block_clear_metadata(&self, block: Address) {
        for metadata_spec in &self.common.metadata.local {
            store_metadata::<VM>(
                &MetadataSpec::OnSide(*metadata_spec),
                unsafe { block.to_object_reference() },
                0,
                None,
                Some(Ordering::SeqCst),
            )
        }
        bzero_alloc_bit(block, BYTES_IN_BLOCK);
    }

    pub fn load_block_tls(&self, block: Address) -> OpaquePointer {
        let tls = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::TLS_TABLE),
            unsafe { block.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        );
        unsafe { std::mem::transmute::<usize, OpaquePointer>(tls) }
    }
}