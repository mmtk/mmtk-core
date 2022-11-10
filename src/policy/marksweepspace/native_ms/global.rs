use std::sync::Arc;

use atomic::Ordering;

use crate::{
    policy::{marksweepspace::native_ms::*, sft::GCWorkerMutRef, space::SpaceOptions},
    scheduler::{GCWorkScheduler, GCWorker},
    util::{
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

use crate::plan::ObjectQueue;
use crate::plan::VectorObjectQueue;
use crate::policy::sft::SFT;
use crate::policy::space::{CommonSpace, Space};
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::chunk_map::*;
use crate::util::linear_scan::Region;
use crate::util::VMThread;
use crate::vm::ObjectModel;
use std::sync::Mutex;

/// The result for `MarkSweepSpace.acquire_block()`. `MarkSweepSpace` will attempt
/// to allocate from abandoned blocks first. If none found, it will get a new block
/// from the page resource.
pub enum BlockAcquireResult {
    Exhausted,
    /// A new block we just acquired from the page resource
    Fresh(Block),
    /// An available block. The block can be directly used if there is any free cell in it.
    AbandonedAvailable(Block),
    /// An unswept block. The block needs to be swept first before it can be used.
    AbandonedUnswept(Block),
}

/// A mark sweep space.
pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    /// Allocation status for all chunks in MS space
    pub chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// Abandoned blocks. If a mutator dies, all its blocks go to this abandoned block
    /// lists. In a GC, we also 'flush' all the local blocks to this global pool so they
    /// can be used by allocators from other threads.
    pub abandoned: Mutex<AbandonedBlockLists>,
}

pub struct AbandonedBlockLists {
    pub available: BlockLists,
    pub unswept: BlockLists,
    pub consumed: BlockLists,
}

impl AbandonedBlockLists {
    fn move_consumed_to_unswept(&mut self) {
        let mut i = 0;
        while i < MI_BIN_FULL {
            if !self.consumed[i].is_empty() {
                self.unswept[i].append(&mut self.consumed[i]);
            }
            i += 1;
        }
    }
}

impl<VM: VMBinding> SFT for MarkSweepSpace<VM> {
    fn name(&self) -> &str {
        self.common.name
    }

    fn is_live(&self, object: crate::util::ObjectReference) -> bool {
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_marked::<VM>(object, Ordering::SeqCst)
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, _object: crate::util::ObjectReference, _alloc: bool) {
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

// We cannot allocate objects that are larger than the max bin size.
#[allow(dead_code)]
pub const MAX_OBJECT_SIZE: usize = crate::policy::marksweepspace::native_ms::MI_LARGE_OBJ_SIZE_MAX;

impl<VM: VMBinding> MarkSweepSpace<VM> {
    pub fn extend_global_side_metadata_specs(_specs: &mut Vec<SideMetadataSpec>) {
        // MarkSweepSpace does not need any special global specs. This method exists, as
        // we need this method for MallocSpace, and we want those two spaces to be used interchangably.
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
        let local_specs = {
            metadata::extract_side_metadata(&vec![
                MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
                MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
                MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::SIZE_TABLE),
                #[cfg(feature = "malloc_native_mimalloc")]
                MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
                #[cfg(feature = "malloc_native_mimalloc")]
                MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
                MetadataSpec::OnSide(Block::BLOCK_LIST_TABLE),
                MetadataSpec::OnSide(Block::TLS_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            ])
        };

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
            abandoned: Mutex::new(AbandonedBlockLists {
                available: new_empty_block_lists(),
                unswept: new_empty_block_lists(),
                consumed: new_empty_block_lists(),
            }),
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
        debug_assert!(
            self.in_space(object),
            "Cannot mark an object {} that was not alloced by free list allocator.",
            object,
        );
        if !VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_marked::<VM>(object, Ordering::SeqCst) {
            VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.mark::<VM>(object, Ordering::SeqCst);
            let block = Block::containing::<VM>(object);
            block.set_state(BlockState::Marked);
            queue.enqueue(object);
        }
        object
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
        // We sweep and release unmarked blocks here. For sweeping cells inside each block, we either
        // do that when we release mutators (eager sweeping), or do that at allocation time (lazy sweeping).
        use crate::scheduler::WorkBucketStage;
        let work_packets = self.generate_sweep_tasks();
        self.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(work_packets);

        let mut abandoned = self.abandoned.lock().unwrap();
        abandoned.move_consumed_to_unswept();
    }

    /// Release a block.
    pub fn release_block(&self, block: Block) {
        self.block_clear_metadata(block);

        block.deinit();
        self.pr.release_pages(block.start());
    }

    pub fn block_clear_metadata(&self, block: Block) {
        for metadata_spec in Block::METADATA_SPECS {
            metadata_spec.set_zero_atomic(block.start(), Ordering::SeqCst);
        }
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::bzero_alloc_bit(block.start(), Block::BYTES);
    }

    pub fn acquire_block(&self, tls: VMThread, size: usize, align: usize) -> BlockAcquireResult {
        {
            let mut abandoned = self.abandoned.lock().unwrap();
            let bin = mi_bin::<VM>(size, align);

            {
                let abandoned_available = &mut abandoned.available;
                if !abandoned_available[bin].is_empty() {
                    let block = Block::from(abandoned_available[bin].pop().unwrap().start());
                    return BlockAcquireResult::AbandonedAvailable(block);
                }
            }

            {
                let abandoned_unswept = &mut abandoned.unswept;
                if !abandoned_unswept[bin].is_empty() {
                    let block = Block::from(abandoned_unswept[bin].pop().unwrap().start());
                    return BlockAcquireResult::AbandonedUnswept(block);
                }
            }
        }

        let acquired = self.acquire(tls, Block::BYTES >> LOG_BYTES_IN_PAGE);
        if acquired.is_zero() {
            BlockAcquireResult::Exhausted
        } else {
            BlockAcquireResult::Fresh(Block::from(acquired))
        }
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
