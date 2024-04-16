use std::sync::Arc;

use atomic::Ordering;

use crate::{
    policy::{marksweepspace::native_ms::*, sft::GCWorkerMutRef},
    scheduler::{GCWorkScheduler, GCWorker},
    util::{
        copy::CopySemantics,
        heap::FreeListPageResource,
        metadata::{self, side_metadata::SideMetadataSpec, MetadataSpec},
        ObjectReference,
    },
    vm::VMBinding,
};

#[cfg(feature = "is_mmtk_object")]
use crate::util::Address;

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
///
/// The space and each free list allocator own some block lists.
/// A block that is in use belongs to exactly one of the block lists. In this case,
/// whoever owns a block list has exclusive access on the blocks in the list.
/// There should be no data race to access blocks. A thread should NOT access a block list
/// if it does not own the block list.
///
/// The table below roughly describes what we do in each phase.
///
/// | Phase          | Allocator local block lists                     | Global abandoned block lists                 | Chunk map |
/// |----------------|-------------------------------------------------|----------------------------------------------|-----------|
/// | Allocation     | Alloc from local                                | Move blocks from global to local block lists | -         |
/// |                | Lazy: sweep local blocks                        |                                              |           |
/// | GC - Prepare   | -                                               | -                                            | Find used chunks, reset block mark, bzero mark bit |
/// | GC - Trace     | Trace object and mark blocks.                   | Trace object and mark blocks.                | -         |
/// |                | No block list access.                           | No block list access.                        |           |
/// | GC - Release   | Lazy: Move blocks to local unswept list         | Lazy: Move blocks to global unswept list     | _         |
/// |                | Eager: Sweep local blocks                       | Eager: Sweep global blocks                   |           |
/// |                | Both: Return local blocks to a temp global list |                                              |           |
/// | GC - End of GC | -                                               | Merge the temp global lists                  | -         |
pub struct MarkSweepSpace<VM: VMBinding> {
    pub common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    /// Allocation status for all chunks in MS space
    chunk_map: ChunkMap,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// Abandoned blocks. If a mutator dies, all its blocks go to this abandoned block
    /// lists. We reuse blocks in these lists in the mutator phase.
    /// The space needs to do the release work for these block lists.
    abandoned: Mutex<AbandonedBlockLists>,
    /// Abandoned blocks during a GC. Each allocator finishes doing release work, and returns
    /// their local blocks to the global lists. Thus we do not need to do release work for
    /// these block lists in the space. These lists are only filled in the release phase,
    /// and will be moved to the abandoned lists above at the end of a GC.
    abandoned_in_gc: Mutex<AbandonedBlockLists>,
}

pub struct AbandonedBlockLists {
    pub available: BlockLists,
    pub unswept: BlockLists,
    pub consumed: BlockLists,
}

impl AbandonedBlockLists {
    fn new() -> Self {
        Self {
            available: new_empty_block_lists(),
            unswept: new_empty_block_lists(),
            consumed: new_empty_block_lists(),
        }
    }

    fn sweep_later<VM: VMBinding>(&mut self, space: &MarkSweepSpace<VM>) {
        for i in 0..MI_BIN_FULL {
            // Release free blocks
            self.available[i].release_blocks(space);
            self.consumed[i].release_blocks(space);
            self.unswept[i].release_blocks(space);

            // Move remaining blocks to unswept
            self.unswept[i].append(&mut self.available[i]);
            self.unswept[i].append(&mut self.consumed[i]);
        }
    }

    fn sweep<VM: VMBinding>(&mut self, space: &MarkSweepSpace<VM>) {
        for i in 0..MI_BIN_FULL {
            self.available[i].release_and_sweep_blocks(space);
            self.consumed[i].release_and_sweep_blocks(space);
            self.unswept[i].release_and_sweep_blocks(space);

            // As we have swept blocks, move blocks in the unswept list to available or consumed list.
            while let Some(block) = self.unswept[i].pop() {
                if block.has_free_cells() {
                    self.available[i].push(block);
                } else {
                    self.consumed[i].push(block);
                }
            }
        }
    }

    fn merge(&mut self, other: &mut Self) {
        for i in 0..MI_BIN_FULL {
            self.available[i].append(&mut other.available[i]);
            self.unswept[i].append(&mut other.unswept[i]);
            self.consumed[i].append(&mut other.consumed[i]);
        }
    }

    #[cfg(debug_assertions)]
    fn assert_empty(&self) {
        for i in 0..MI_BIN_FULL {
            assert!(self.available[i].is_empty());
            assert!(self.unswept[i].is_empty());
            assert!(self.consumed[i].is_empty());
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

    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, _object: ObjectReference) -> bool {
        false
    }

    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        false
    }

    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        false
    }

    fn is_movable(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }

    fn initialize_object_metadata(&self, _object: crate::util::ObjectReference, _alloc: bool) {
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit::<VM>(_object);
    }

    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr::<VM>(addr).is_some()
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

    fn initialize_sft(&self, sft_map: &mut dyn crate::policy::sft_map::SFTMap) {
        self.common().initialize_sft(self.as_sft(), sft_map)
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
    // Allow ptr_arg as we want to keep the function signature the same as for malloc marksweep
    #[allow(clippy::ptr_arg)]
    pub fn extend_global_side_metadata_specs(_specs: &mut Vec<SideMetadataSpec>) {
        // MarkSweepSpace does not need any special global specs. This method exists, as
        // we need this method for MallocSpace, and we want those two spaces to be used interchangably.
    }

    pub fn new(args: crate::policy::space::PlanCreateSpaceArgs<VM>) -> MarkSweepSpace<VM> {
        let scheduler = args.scheduler.clone();
        let vm_map = args.vm_map;
        let is_discontiguous = args.vmrequest.is_discontiguous();
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
        let common = CommonSpace::new(args.into_policy_args(false, false, local_specs));
        MarkSweepSpace {
            pr: if is_discontiguous {
                FreeListPageResource::new_discontiguous(vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, vm_map)
            },
            common,
            chunk_map: ChunkMap::new(),
            scheduler,
            abandoned: Mutex::new(AbandonedBlockLists::new()),
            abandoned_in_gc: Mutex::new(AbandonedBlockLists::new()),
        }
    }

    fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(!object.is_null());
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

    pub fn prepare(&mut self) {
        #[cfg(debug_assertions)]
        self.abandoned_in_gc.lock().unwrap().assert_empty();

        // # Safety: MarkSweepSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        let work_packets = self
            .chunk_map
            .generate_tasks(|chunk| Box::new(PrepareChunkMap { space, chunk }));
        self.scheduler.work_buckets[crate::scheduler::WorkBucketStage::Prepare]
            .bulk_add(work_packets);
    }

    pub fn release(&mut self) {
        if cfg!(feature = "eager_sweeping") {
            // For eager sweeping, we have to sweep the lists that are abandoned to these global lists.
            let mut abandoned = self.abandoned.lock().unwrap();
            abandoned.sweep(self);
        } else {
            // For lazy sweeping, we just move blocks from consumed to unswept. When an allocator tries
            // to use them, they will sweep the block.
            let mut abandoned = self.abandoned.lock().unwrap();
            abandoned.sweep_later(self);
        }
    }

    pub fn end_of_gc(&mut self) {
        let from = self.abandoned_in_gc.get_mut().unwrap();
        let to = self.abandoned.get_mut().unwrap();
        to.merge(from);

        #[cfg(debug_assertions)]
        from.assert_empty();
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
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::bzero_vo_bit(block.start(), Block::BYTES);
    }

    pub fn acquire_block(&self, tls: VMThread, size: usize, align: usize) -> BlockAcquireResult {
        {
            let mut abandoned = self.abandoned.lock().unwrap();
            let bin = mi_bin::<VM>(size, align);

            {
                let abandoned_available = &mut abandoned.available;
                if !abandoned_available[bin].is_empty() {
                    let block = abandoned_available[bin].pop().unwrap();
                    return BlockAcquireResult::AbandonedAvailable(block);
                }
            }

            {
                let abandoned_unswept = &mut abandoned.unswept;
                if !abandoned_unswept[bin].is_empty() {
                    let block = abandoned_unswept[bin].pop().unwrap();
                    return BlockAcquireResult::AbandonedUnswept(block);
                }
            }
        }

        let acquired = self.acquire(tls, Block::BYTES >> LOG_BYTES_IN_PAGE);
        if acquired.is_zero() {
            BlockAcquireResult::Exhausted
        } else {
            BlockAcquireResult::Fresh(Block::from_unaligned_address(acquired))
        }
    }

    pub fn get_abandoned_block_lists(&self) -> &Mutex<AbandonedBlockLists> {
        &self.abandoned
    }

    pub fn get_abandoned_block_lists_in_gc(&self) -> &Mutex<AbandonedBlockLists> {
        &self.abandoned_in_gc
    }
}

use crate::scheduler::GCWork;
use crate::MMTK;

struct PrepareChunkMap<VM: VMBinding> {
    space: &'static MarkSweepSpace<VM>,
    chunk: Chunk,
}

impl<VM: VMBinding> GCWork<VM> for PrepareChunkMap<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        debug_assert!(self.space.chunk_map.get(self.chunk) == ChunkState::Allocated);
        // number of allocated blocks.
        let mut n_occupied_blocks = 0;
        self.chunk
            .iter_region::<Block>()
            .filter(|block| block.get_state() != BlockState::Unallocated)
            .for_each(|block| {
                // Clear block mark
                block.set_state(BlockState::Unmarked);
                // Count occupied blocks
                n_occupied_blocks += 1
            });
        if n_occupied_blocks == 0 {
            // Set this chunk as free if there is no live blocks.
            self.space.chunk_map.set(self.chunk, ChunkState::Free)
        } else {
            // Otherwise this chunk is occupied, and we reset the mark bit if it is on the side.
            if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
                for chunk in self.space.chunk_map.all_chunks() {
                    side.bzero_metadata(chunk.start(), Chunk::BYTES);
                }
            }
        }
    }
}
