use super::chunk_map::Chunk;
use super::pageresource::{PRAllocFail, PRAllocResult};
use super::{FreeListPageResource, PageResource};
use crate::policy::immix::block::{Block, BlockState};
use crate::policy::space::Space;
use crate::util::address::Address;
use crate::util::constants::*;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::layout::VMMap;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::linear_scan::Region;
use crate::util::memory::MmapStrategy;
use crate::util::metadata::side_metadata::spec_defs::{BLOCK_IN_USE, BLOCK_OWNER};
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::opaque_pointer::*;
use crate::vm::*;
use atomic::Ordering;
use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::RwLock;

/// A fast PageResource for fixed-size block allocation only.
pub struct BlockPageResource<VM: VMBinding, B: Region + 'static> {
    flpr: FreeListPageResource<VM>,
    pub(crate) total_chunks: AtomicUsize,
    chunks: RwLock<Vec<Chunk>>,
    clean_block_cursor: AtomicUsize,
    clean_block_steal_cursor: AtomicUsize,
    reuse_block_cursor: AtomicUsize,
    reuse_block_steal_cursor: AtomicUsize,
    clean_block_cursor_before_gc: AtomicUsize,
    reuse_block_cursor_before_gc: AtomicUsize,
    _p: PhantomData<B>,
    rc_enabled: bool,
}

impl<VM: VMBinding, B: Region> PageResource<VM> for BlockPageResource<VM, B> {
    fn common(&self) -> &CommonPageResource {
        self.flpr.common()
    }

    fn common_mut(&mut self) -> &mut CommonPageResource {
        self.flpr.common_mut()
    }

    fn alloc_pages(
        &self,
        _space: &dyn Space<VM>,
        _reserved_pages: usize,
        _required_pages: usize,
        _tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        unimplemented!()
    }

    fn get_available_physical_pages(&self) -> usize {
        self.flpr.get_available_physical_pages()
    }

    fn has_chunk_fragmentation_info(&self) -> bool {
        false
    }

    fn get_live_pages_in_chunk(&self, _: Chunk) -> usize {
        0
    }
}

impl<VM: VMBinding, B: Region> BlockPageResource<VM, B> {
    /// Block granularity in pages
    const LOG_PAGES: usize = B::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    const BLOCKS_IN_CHUNK: usize = 1 << (Chunk::LOG_BYTES - B::LOG_BYTES);

    fn append_local_metadata(metadata: &mut SideMetadataContext) {
        metadata.local.push(BLOCK_IN_USE);
        metadata.local.push(BLOCK_OWNER);
    }

    pub(crate) fn new_contiguous(
        log_pages: usize,
        start: Address,
        bytes: usize,
        vm_map: &'static dyn VMMap,
        _num_workers: usize,
        mut metadata: SideMetadataContext,
    ) -> Self {
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self::append_local_metadata(&mut metadata);
        Self {
            flpr: FreeListPageResource::new_contiguous(start, bytes, vm_map, metadata),
            total_chunks: AtomicUsize::new(0),
            chunks: RwLock::new(vec![]),
            clean_block_cursor: AtomicUsize::new(0),
            clean_block_steal_cursor: AtomicUsize::new(0),
            reuse_block_cursor: AtomicUsize::new(0),
            reuse_block_steal_cursor: AtomicUsize::new(0),
            clean_block_cursor_before_gc: AtomicUsize::new(0),
            reuse_block_cursor_before_gc: AtomicUsize::new(0),
            rc_enabled: false,
            _p: PhantomData,
        }
    }

    pub(crate) fn new_discontiguous(
        log_pages: usize,
        vm_map: &'static dyn VMMap,
        _num_workers: usize,
        mut metadata: SideMetadataContext,
    ) -> Self {
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self::append_local_metadata(&mut metadata);
        Self {
            flpr: FreeListPageResource::new_discontiguous(vm_map, metadata),
            total_chunks: AtomicUsize::new(0),
            chunks: RwLock::new(vec![]),
            clean_block_cursor: AtomicUsize::new(0),
            clean_block_steal_cursor: AtomicUsize::new(0),
            reuse_block_cursor: AtomicUsize::new(0),
            reuse_block_steal_cursor: AtomicUsize::new(0),
            clean_block_cursor_before_gc: AtomicUsize::new(0),
            reuse_block_cursor_before_gc: AtomicUsize::new(0),
            rc_enabled: false,
            _p: PhantomData,
        }
    }

    pub(crate) fn rc(mut self, lxr: bool) -> Self {
        self.rc_enabled = lxr;
        self
    }

    fn block_index_to_block(&self, chunks: &[Chunk], i: usize) -> B {
        let c_index = i >> (Chunk::LOG_BYTES - B::LOG_BYTES);
        let chunk = chunks[c_index];
        let block = B::from_aligned_address(
            chunk.start() + ((i & (Self::BLOCKS_IN_CHUNK - 1)) << B::LOG_BYTES),
        );
        block
    }

    fn block_is_in_defrag_source(&self, copy: bool, block: Block) -> bool {
        if !self.rc_enabled {
            // Not LXR.
            // For mutator-allocators, we will never visit defrag source blocks.
            return copy && block.is_defrag_source();
        } else {
            // LXR.
            return block.is_defrag_source();
        }
    }

    /// Check if a block is available for allocation
    fn block_is_available(
        &self,
        block: B,
        clean: bool,
        copy: bool,
        mature_evac: bool,
        _owner: VMThread,
    ) -> bool {
        let b = Block::from_aligned_address(block.start());
        let state = b.get_state();
        if !clean {
            return state != BlockState::Unallocated
                && !b.is_reusing()
                && (!copy || !b.is_gc_reusing())
                && !self.block_is_in_defrag_source(copy, b)
                && (copy || b.get_owner().is_none());
        }
        // Don't allocate into a non-empty block
        if state != BlockState::Unallocated {
            return false;
        }
        // Copy allocator: Skip young blocks in the previous mutator phase
        if copy && !mature_evac {
            return !b.is_nursery();
        } else if copy {
            return true;
        }
        // Mutator allocator: Skip blocks owned by other mutators. We need to steal instead.
        // We only allocate clean blocks without an owner. For owned blocks, we need to steal them.
        b.get_owner().is_none()
    }

    /// Check if a block can be safely stolen from it's owner
    fn block_is_stealable(
        &self,
        block: B,
        clean: bool,
        owner: VMThread,
        skip_lock_check: bool,
        copy: bool,
    ) -> bool {
        let block = Block::from_aligned_address(block.start());
        if clean {
            let state = block.get_state();
            // Don't steal non-empty blocks
            if state != BlockState::Unallocated {
                return false;
            }
            let block_owner = block.get_owner();
            if block_owner.is_none() || block_owner == Some(owner) {
                return false;
            }
            // Not filled by a mutator in the last mutator phase
            !block.is_nursery()
        // in_use state is not set
            && (skip_lock_check || !block.is_locked())
        } else {
            let state = block.get_state();
            // Don't steal empty, used, or defrag blocks
            if state == BlockState::Unallocated
                || block.is_reusing()
                || self.block_is_in_defrag_source(copy, block)
            {
                return false;
            }
            let block_owner = block.get_owner();
            if block_owner.is_none() || block_owner == Some(owner) {
                return false;
            }
            skip_lock_check || !block.is_locked()
        }
    }

    // We successfully allocated or stole a block. Now add it to the local block list.
    fn append_to_buf(
        &self,
        buf: &mut Vec<B>,
        block: B,
        copy: bool,
        mature_evac: bool,
        steal: bool,
        owner: VMThread,
        clean: bool,
    ) {
        let b = Block::from_aligned_address(block.start());
        if !steal {
            if !copy {
                let locked = b.try_lock_with_condition(|| {
                    self.block_is_available(block, clean, copy, mature_evac, owner)
                });
                if locked {
                    b.set_owner(Some(owner));
                    buf.push(block);
                    b.unlock();
                }
            } else {
                if clean {
                    if b.get_state() != BlockState::Unallocated || (!mature_evac && b.is_nursery())
                    {
                        // debug_assert!(!b.is_defrag_source());
                        return;
                    }
                } else {
                    if b.get_state() == BlockState::Unallocated
                        || b.is_reusing()
                        || self.block_is_in_defrag_source(copy, b)
                    {
                        return;
                    }
                }
                buf.push(block);
            }
        } else {
            buf.push(block);
        }
    }

    // Attempt to steal a block.
    fn attempt_to_steal(&self, block: B, owner: VMThread, clean: bool, copy: bool) -> bool {
        // Attempt to set as in-use
        let b = Block::from_aligned_address(block.start());
        let locked =
            b.try_lock_with_condition(|| self.block_is_stealable(block, clean, owner, true, copy));
        if !locked {
            return false;
        }
        // Set owner
        b.set_owner(Some(owner));
        // clear in-use
        b.unlock();
        true
    }

    fn acquire_clean_blocks_fast(
        &self,
        count: usize,
        buf: &mut Vec<B>,
        chunks: &Vec<Chunk>,
        copy: bool,
        mature_evac: bool,
        owner: VMThread,
    ) -> bool {
        // linear scan the chunks to find a reusable block
        let max_b_index = chunks.len() << (Chunk::LOG_BYTES - B::LOG_BYTES);
        let b_index = self.clean_block_cursor.load(Ordering::Relaxed);
        // Bail out if we don't have any blocks to allocate
        if b_index >= max_b_index {
            return false;
        }
        // Grab 1~N Blocks
        let mut new_cursor = 0;
        let mut actual_count = 0;
        let old = self
            .clean_block_cursor
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                let mut i = c;
                let mut curr_count = 0;
                while i < max_b_index {
                    let block = self.block_index_to_block(chunks, i);
                    i += 1;
                    if self.block_is_available(block, true, copy, mature_evac, owner) {
                        curr_count += 1;
                        if curr_count >= count {
                            break;
                        }
                    }
                }
                new_cursor = i;
                actual_count = curr_count;
                if i != c {
                    Some(i)
                } else {
                    None
                }
            });
        if actual_count != 0 {
            let old = old.unwrap();
            for i in old..usize::min(new_cursor, max_b_index) {
                let block = self.block_index_to_block(chunks, i);
                if self.block_is_available(block, true, copy, mature_evac, owner) {
                    self.append_to_buf(buf, block, copy, mature_evac, false, owner, true);
                }
            }
            true
        } else {
            false
        }
    }

    fn acquire_reusable_blocks_fast(
        &self,
        count: usize,
        buf: &mut Vec<B>,
        chunks: &Vec<Chunk>,
        copy: bool,
        mature_evac: bool,
        owner: VMThread,
    ) -> bool {
        // linear scan the chunks to find a reusable block
        let max_b_index = chunks.len() << (Chunk::LOG_BYTES - B::LOG_BYTES);
        let b_index = self.reuse_block_cursor.load(Ordering::Relaxed);
        // Bail out if we don't have any blocks to allocate
        if b_index >= max_b_index {
            return false;
        }
        // Grab 1~N non-empty Blocks
        let mut new_cursor = 0;
        let mut actual_count = 0;
        let old = self
            .reuse_block_cursor
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                let mut i = c;
                let mut curr_count = 0;
                while i < max_b_index {
                    let block = self.block_index_to_block(chunks, i);
                    i += 1;
                    if self.block_is_available(block, false, copy, mature_evac, owner) {
                        curr_count += 1;
                        if curr_count >= count {
                            break;
                        }
                    }
                }
                new_cursor = i;
                actual_count = curr_count;
                if i != c {
                    Some(i)
                } else {
                    None
                }
            });
        if actual_count != 0 {
            let old = old.unwrap();
            for i in old..usize::min(new_cursor, max_b_index) {
                let block = self.block_index_to_block(chunks, i);
                if self.block_is_available(block, false, copy, mature_evac, owner) {
                    self.append_to_buf(buf, block, copy, mature_evac, false, owner, false);
                }
            }
            true
        } else {
            false
        }
    }

    fn steal_reusable_blocks(
        &self,
        count: usize,
        buf: &mut Vec<B>,
        chunks: &Vec<Chunk>,
        copy: bool,
        owner: VMThread,
    ) -> bool {
        // linear scan the chunks to find a reusable block
        let b_index = self.reuse_block_steal_cursor.load(Ordering::Relaxed);
        // Bail out if we don't have any blocks to allocate
        if b_index == 0 || copy {
            return false;
        }
        // Grab 1~N Blocks
        let mut new_cursor = 0;
        let mut actual_count = 0;
        let old =
            self.reuse_block_steal_cursor
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                    let mut i = c;
                    let mut curr_count = 0;
                    while i > 0 {
                        let block = self.block_index_to_block(chunks, i - 1);
                        i -= 1;
                        if self.block_is_stealable(block, false, owner, false, copy) {
                            curr_count += 1;
                            if curr_count >= count {
                                break;
                            }
                        }
                    }
                    new_cursor = i;
                    actual_count = curr_count;
                    if i != c {
                        Some(i)
                    } else {
                        None
                    }
                });
        if actual_count != 0 {
            let old = old.unwrap();
            for i in new_cursor..old {
                let block = self.block_index_to_block(chunks, i);
                if self.block_is_stealable(block, false, owner, false, copy) {
                    if self.attempt_to_steal(block, owner, false, copy) {
                        self.append_to_buf(buf, block, copy, false, true, owner, false);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    fn steal_clean_blocks(
        &self,
        count: usize,
        buf: &mut Vec<B>,
        chunks: &Vec<Chunk>,
        copy: bool,
        owner: VMThread,
    ) -> bool {
        // linear scan the chunks to find a reusable block
        let b_index = self.clean_block_steal_cursor.load(Ordering::Relaxed);
        // Bail out if we don't have any blocks to allocate
        if b_index == 0 || copy {
            return false;
        }
        // Grab 1~N Blocks
        let mut new_cursor = 0;
        let mut actual_count = 0;
        let old =
            self.clean_block_steal_cursor
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                    let mut i = c;
                    let mut curr_count = 0;
                    while i > 0 {
                        let block = self.block_index_to_block(chunks, i - 1);
                        i -= 1;
                        if self.block_is_stealable(block, true, owner, false, copy) {
                            curr_count += 1;
                            if curr_count >= count {
                                break;
                            }
                        }
                    }
                    new_cursor = i;
                    actual_count = curr_count;
                    if i != c {
                        Some(i)
                    } else {
                        None
                    }
                });
        if actual_count != 0 {
            let old = old.unwrap();
            for i in new_cursor..old {
                let block = self.block_index_to_block(chunks, i);
                if self.block_is_stealable(block, true, owner, false, copy) {
                    if self.attempt_to_steal(block, owner, true, copy) {
                        self.append_to_buf(buf, block, copy, false, true, owner, true);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    pub fn acquire_blocks(
        &self,
        alloc_count: usize,
        steal_count: usize,
        clean: bool,
        buf: &mut Vec<B>,
        space: &dyn Space<VM>,
        copy: bool,
        mature_evac: bool,
        owner: VMThread,
    ) -> bool {
        let chunks = self.chunks.read().unwrap();
        if !clean {
            if self.acquire_reusable_blocks_fast(
                alloc_count,
                buf,
                &*chunks,
                copy,
                mature_evac,
                owner,
            ) {
                return true;
            }
            return self.steal_reusable_blocks(steal_count, buf, &*chunks, copy, owner);
        }
        if self.acquire_clean_blocks_fast(alloc_count, buf, &*chunks, copy, mature_evac, owner) {
            return true;
        }
        if self.steal_clean_blocks(steal_count, buf, &*chunks, copy, owner) {
            return true;
        }
        // Slow path
        std::mem::drop(chunks);
        let mut chunks = self.chunks.write().unwrap();
        if self.acquire_clean_blocks_fast(alloc_count, buf, &*chunks, copy, mature_evac, owner) {
            return true;
        }
        if self.steal_clean_blocks(steal_count, buf, &*chunks, copy, owner) {
            return true;
        }
        // 1. Get a new chunk
        let chunk = self.alloc_chunk(space).unwrap();
        gc_log!([3] "new-chunk: {:?} (total={})", chunk.start(), chunks.len());
        // 2. Take the first N blocks in the chunk as the allocation result
        let count = usize::min(alloc_count, Self::BLOCKS_IN_CHUNK);
        for i in 0..count {
            let block = B::from_aligned_address(
                chunk.start() + ((i & (Self::BLOCKS_IN_CHUNK - 1)) << B::LOG_BYTES),
            );
            self.append_to_buf(buf, block, copy, mature_evac, false, owner, true);
        }
        // 3. Add the chunk to the chunk list
        let total_blocks = chunks.len() * Self::BLOCKS_IN_CHUNK;
        chunks.push(chunk);
        self.clean_block_cursor
            .store(total_blocks + count, Ordering::SeqCst);
        true
    }

    fn alloc_chunk(&self, space: &dyn Space<VM>) -> Option<Chunk> {
        let start = self
            .common()
            .grow_discontiguous_space(space.common().descriptor, 1, None);
        if start.is_zero() {
            return None;
        }
        if let Err(mmap_error) = crate::mmtk::MMAPPER
            .ensure_mapped(start, PAGES_IN_CHUNK as _, MmapStrategy::INTERNAL_MEMORY)
            .and(
                self.common()
                    .metadata
                    .try_map_metadata_space(start, BYTES_IN_CHUNK),
            )
        {
            crate::util::memory::handle_mmap_error::<VM>(mmap_error, VMThread::UNINITIALIZED);
        }
        space.grow_space(start, BYTES_IN_CHUNK, true);
        self.total_chunks.fetch_add(1, Ordering::SeqCst);
        Some(Chunk::from_aligned_address(start))
    }

    pub fn flush_all(&self) {}

    pub fn available_pages(&self) -> usize {
        let total = self.total_chunks.load(Ordering::SeqCst)
            << (LOG_BYTES_IN_CHUNK - LOG_BYTES_IN_PAGE as usize);
        total.saturating_sub(self.reserved_pages())
    }

    pub fn bulk_release_blocks(&self, count: usize) {
        // gc_log!("Bulk release blocks {}", count);
        let pages = count << Self::LOG_PAGES;
        debug_assert!(pages as usize <= self.common().accounting.get_committed_pages());
        self.common().accounting.release(pages as _);
        // gc_log!("Bulk release blocks {}", count);
    }

    pub fn exhausted_reusable_space(&self) -> bool {
        let chunks = self.chunks.read().unwrap();
        let max_b_index = chunks.len() << (Chunk::LOG_BYTES - B::LOG_BYTES);
        self.reuse_block_cursor.load(Ordering::Relaxed) >= max_b_index
            && self.reuse_block_steal_cursor.load(Ordering::Relaxed) == 0
    }

    pub fn prepare_gc(&self) {
        let _chunks = self.chunks.write().unwrap();
        self.clean_block_cursor.store(0, Ordering::SeqCst);
        self.reuse_block_cursor.store(0, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        let chunks = self.chunks.write().unwrap();
        let max_b_index = chunks.len() << (Chunk::LOG_BYTES - B::LOG_BYTES);
        self.clean_block_cursor.store(0, Ordering::SeqCst);
        self.clean_block_steal_cursor
            .store(max_b_index, Ordering::SeqCst);
        self.reuse_block_cursor.store(0, Ordering::SeqCst);
        self.reuse_block_steal_cursor
            .store(max_b_index, Ordering::SeqCst);
    }

    pub fn reset_before_mature_evac(&self) {
        let _chunks = self.chunks.write().unwrap();
        self.clean_block_cursor.store(0, Ordering::SeqCst);
        self.reuse_block_cursor.store(0, Ordering::SeqCst);
    }

    pub fn reset_nursery_state(&self) {
        let chunks = self.chunks.write().unwrap();
        for c in &*chunks {
            Block::PHASE_EPOCH.bzero_metadata(c.start(), Chunk::BYTES);
        }
    }
}
