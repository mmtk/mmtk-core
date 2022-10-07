use std::sync::Arc;

use atomic::Ordering;

use crate::{
    policy::{
        marksweepspace::{
            block::{Block, BlockState},
            metadata::{is_marked, set_mark_bit},
        },
        sft::GCWorkerMutRef,
        space::SpaceOptions,
    },
    scheduler::{GCWorkScheduler, GCWorker},
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

use super::super::space::{CommonSpace, Space};
use crate::plan::ObjectQueue;
use crate::plan::VectorObjectQueue;
use crate::policy::sft::SFT;
use crate::util::alloc::free_list_allocator::{new_empty_block_lists, BlockLists};
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::chunk_map::*;
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
            abandoned_available: Mutex::from(new_empty_block_lists()),
            abandoned_unswept: Mutex::from(new_empty_block_lists()),
            abandoned_consumed: Mutex::from(new_empty_block_lists()),
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

    pub fn prepare(&mut self) {
        if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
            for chunk in self.chunk_map.all_chunks() {
                side.bzero_metadata(chunk.start(), Chunk::BYTES);
            }
        } else {
            unimplemented!("in header mark bit is not supported");
        }
    }

    pub fn release(&mut self) {
        use crate::scheduler::WorkBucketStage;
        use crate::util::alloc::free_list_allocator::MI_BIN_FULL;
        let work_packets = self.generate_sweep_tasks();
        self.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
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

    pub fn generate_sweep_tasks(&self) -> Vec<Box<dyn GCWork<VM>>> {
        // # Safety: ImmixSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        self.chunk_map
            .generate_tasks(|chunk| Box::new(SweepChunk { space, chunk }))
    }
}

use crate::scheduler::GCWork;
use crate::MMTK;

/// Chunk sweeping work packet.
struct SweepChunk<VM: VMBinding> {
    space: &'static MarkSweepSpace<VM>,
    chunk: Chunk,
}

impl<VM: VMBinding> GCWork<VM> for SweepChunk<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        debug_assert!(self.space.chunk_map.get(self.chunk) == ChunkState::Allocated);
        // number of allocated blocks.
        let mut allocated_blocks = 0;
        // Iterate over all allocated blocks in this chunk.
        for block in self
            .chunk
            .iter_region::<Block>()
            .filter(|block| block.get_state() != BlockState::Unallocated)
        {
            if !block.attempt_release(self.space) {
                // Block is live. Increment the allocated block count.
                allocated_blocks += 1;
            }
        }
        // Set this chunk as free if there is not live blocks.
        if allocated_blocks == 0 {
            self.space.chunk_map.set(self.chunk, ChunkState::Free)
        }
    }
}
