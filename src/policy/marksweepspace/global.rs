use std::sync::Arc;

use atomic::Ordering;

use crate::{
    policy::{
        marksweepspace::{
            block::{Block, BlockState},
            chunk::Chunk,
            metadata::{is_marked, set_mark_bit},
        },
        sft::GCWorkerMutRef,
        space::SpaceOptions,
    },
    scheduler::{GCWorkScheduler, GCWorker, WorkBucketStage},
    util::{
        alloc::free_list_allocator::mi_bin,
        alloc_bit::is_alloced,
        copy::CopySemantics,
        heap::{
            layout::heap_layout::{Mmapper, VMMap},
            FreeListPageResource, HeapMeta, VMRequest,
        },
        metadata::{
            self,
            side_metadata::{SideMetadataContext, SideMetadataSpec},
            MetadataSpec,
        },
        ObjectReference,
    },
    vm::VMBinding,
};

use super::{
    super::space::{CommonSpace, Space},
    chunk::{ChunkMap, ChunkState},
};
use crate::plan::ObjectQueue;
use crate::plan::VectorObjectQueue;
use crate::policy::sft::SFT;
use crate::util::alloc::free_list_allocator::MI_BIN_FULL;
use crate::util::alloc::free_list_allocator::{BlockLists, BLOCK_LISTS_EMPTY};
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::linear_scan::Region;
use crate::util::VMThread;
use crate::vm::ObjectModel;
use std::sync::Mutex;

pub enum BlockAcquireResult {
    Fresh(Block),
    AbandonedAvailable(Block),
    AbandonedUnswept(Block),
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
        "MarkSweepSpace"
    }

    fn is_live(&self, object: crate::util::ObjectReference) -> bool {
        is_marked::<VM>(object, Ordering::SeqCst)
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, _object: crate::util::ObjectReference, _alloc: bool) {
        // do nothing
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit(_object);
    }

    fn sft_trace_object(
        &self,
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        self.trace_object(queue, object)
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

    fn initialize_sft(&self) {
        self.common().initialize_sft(self.as_sft())
    }

    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }

    fn release_multiple_pages(&mut self, _start: crate::util::Address) {
        todo!()
    }
}

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for MarkSweepSpace<VM> {
    fn trace_object<Q: ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _copy: Option<CopySemantics>,
        _worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.trace_object(queue, object)
    }

    fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
        false
    }
}

impl<VM: VMBinding> MarkSweepSpace<VM> {
    /// Get work packet scheduler
    fn scheduler(&self) -> &GCWorkScheduler<VM> {
        &self.scheduler
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> MarkSweepSpace<VM> {
        // FIXME: alloc bit should be optional
        // let alloc_bits =
        //     &mut metadata::extract_side_metadata(&[MetadataSpec::OnSide(ALLOC_SIDE_METADATA_SPEC)]);

        // let mark_bits =
        //     &mut metadata::extract_side_metadata(&[*VM::VMObjectModel::LOCAL_MARK_BIT_SPEC]);

        let local_specs = {
            metadata::extract_side_metadata(&vec![
                MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
                MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
                MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::SIZE_TABLE),
                // MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
                // MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::BLOCK_LIST_TABLE),
                MetadataSpec::OnSide(Block::TLS_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            ])
        };

        // local_specs.append(mark_bits);

        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: false,
                immortal: false,
                needs_log_bit: false,
                zeroed,
                vmrequest,
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: local_specs,
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        MarkSweepSpace {
            pr: if vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, vm_map)
            },
            common,
            chunk_map: ChunkMap::new(),
            scheduler,
            abandoned_available: Mutex::from(BLOCK_LISTS_EMPTY),
            abandoned_unswept: Mutex::from(BLOCK_LISTS_EMPTY),
            abandoned_consumed: Mutex::from(BLOCK_LISTS_EMPTY),
        }
    }

    fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
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
        if !is_marked::<VM>(object, Ordering::SeqCst) {
            set_mark_bit::<VM>(object, Ordering::SeqCst);
            let block = Block::from(Block::align(address));
            block.set_state(BlockState::Marked);
            queue.enqueue(object);
        }
        object
    }

    pub fn zero_mark_bits(&self) {
        // todo: concurrent zeroing
        use crate::vm::*;
        for chunk in self.chunk_map.all_chunks() {
            if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
                side.bzero_metadata(chunk.start(), Chunk::BYTES);
            } else {
                unimplemented!();
            }
        }
    }

    pub fn block_has_no_objects(&self, block: Block) -> bool {
        // for debugging, delete this later
        // assumes block is allocated (has metadata)
        let size = block.load_block_cell_size();
        let mut cell = block.start();
        while cell < block.start() + Block::BYTES {
            if is_alloced(unsafe { cell.to_object_reference() }) {
                return false;
            }
            cell += size;
        }
        true
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
        let work_packets = self.chunk_map.generate_sweep_tasks(space);
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
        let mut abandoned_unswept = self.abandoned_unswept.lock().unwrap();
        let mut abandoned_consumed = self.abandoned_consumed.lock().unwrap();
        let mut i = 0;
        while i < MI_BIN_FULL {
            if !abandoned_consumed[i].is_empty() {
                abandoned_consumed[i].lock();
                abandoned_unswept[i].lock();
                abandoned_unswept[i].append(&mut abandoned_consumed[i]);
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
        let clear_metadata = |spec: &SideMetadataSpec| match spec.log_num_of_bits {
            0..=3 => spec.store_atomic::<u8>(block.start(), 0, Ordering::SeqCst),
            4 => spec.store_atomic::<u16>(block.start(), 0, Ordering::SeqCst),
            5 => spec.store_atomic::<u32>(block.start(), 0, Ordering::SeqCst),
            6 => spec.store_atomic::<u64>(block.start(), 0, Ordering::SeqCst),
            _ => unreachable!(),
        };
        for metadata_spec in &self.common.metadata.local {
            // FIXME: is all local metadata based on block?
            clear_metadata(metadata_spec);
        }
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::bzero_alloc_bit(block.start(), Block::BYTES);
    }

    pub fn acquire_block(&self, tls: VMThread, size: usize, align: usize) -> BlockAcquireResult {
        let bin = mi_bin::<VM>(size, align);

        {
            let mut abandoned = self.abandoned_available.lock().unwrap();
            if !abandoned[bin].is_empty() {
                let block = Block::from(abandoned[bin].pop().start());
                return BlockAcquireResult::AbandonedAvailable(block);
            }
        }

        {
            let mut abandoned_unswept = self.abandoned_unswept.lock().unwrap();
            if !abandoned_unswept[bin].is_empty() {
                let block = Block::from(abandoned_unswept[bin].pop().start());
                return BlockAcquireResult::AbandonedUnswept(block);
            }
        }
        BlockAcquireResult::Fresh(Block::from(
            self.acquire(tls, Block::BYTES >> LOG_BYTES_IN_PAGE),
        ))
    }
}
