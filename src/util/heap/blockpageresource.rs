use super::chunk_map::Chunk;
use super::pageresource::{PRAllocFail, PRAllocResult};
use super::{FreeListPageResource, PageResource};
use crate::policy::space::Space;
use crate::util::address::Address;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::layout::VMMap;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::linear_scan::Region;
use crate::util::memory::MmapStrategy;
use crate::util::metadata::side_metadata::spec_defs::CHUNK_LIVE_BLOCKS;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::opaque_pointer::*;
use crate::util::{constants::*, memory};
use crate::vm::*;
use atomic::{Atomic, Ordering};
use crossbeam::queue::SegQueue;
use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

/// A fast PageResource for fixed-size block allocation only.
pub struct BlockPageResource<VM: VMBinding, B: Region + 'static> {
    flpr: FreeListPageResource<VM>,
    block_alloc: LockFreeListBlockAlloc<B>,
    chunk_queue: SegQueue<Chunk>,
    sync: Mutex<()>,
    pub(crate) total_chunks: AtomicUsize,
    _p: PhantomData<B>,
}

impl<VM: VMBinding, B: Region> PageResource<VM> for BlockPageResource<VM, B> {
    fn common(&self) -> &CommonPageResource {
        self.flpr.common()
    }

    fn common_mut(&mut self) -> &mut CommonPageResource {
        self.flpr.common_mut()
    }

    fn update_discontiguous_start(&mut self, start: Address) {
        self.flpr.update_discontiguous_start(start)
    }

    fn alloc_pages(
        &self,
        space: &dyn Space<VM>,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        if let Some((block, new_chunk)) = self.block_alloc.alloc(self, space) {
            CHUNK_LIVE_BLOCKS.fetch_add_atomic(Chunk::align(block.start()), 1u16, Ordering::SeqCst);
            self.commit_pages(reserved_pages, required_pages, tls);
            Ok(PRAllocResult {
                start: block.start(),
                pages: required_pages,
                new_chunk,
            })
        } else {
            Err(PRAllocFail)
        }
    }

    fn get_available_physical_pages(&self) -> usize {
        let _sync = self.sync.lock().unwrap();
        self.flpr.get_available_physical_pages()
    }

    fn has_chunk_fragmentation_info(&self) -> bool {
        !cfg!(feature = "lxr_no_chunk_defrag")
    }

    fn get_live_pages_in_chunk(&self, c: Chunk) -> usize {
        (CHUNK_LIVE_BLOCKS.load_atomic::<u16>(c.start(), Ordering::Relaxed) as usize)
            << Self::LOG_PAGES
    }
}

impl<VM: VMBinding, B: Region> BlockPageResource<VM, B> {
    /// Block granularity in pages
    const LOG_PAGES: usize = B::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    const BLOCKS_IN_CHUNK: usize = 1 << (Chunk::LOG_BYTES - B::LOG_BYTES);

    fn append_local_metadata(metadata: &mut SideMetadataContext) {
        metadata.local.push(CHUNK_LIVE_BLOCKS);
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
            block_alloc: <LockFreeListBlockAlloc<B> as BlockAlloc<VM, B>>::new(),
            sync: Mutex::default(),
            chunk_queue: SegQueue::new(),
            total_chunks: AtomicUsize::new(0),
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
            block_alloc: <LockFreeListBlockAlloc<B> as BlockAlloc<VM, B>>::new(),
            sync: Mutex::default(),
            chunk_queue: SegQueue::new(),
            total_chunks: AtomicUsize::new(0),
            _p: PhantomData,
        }
    }

    fn alloc_chunk(&self, space: &dyn Space<VM>) -> Option<Chunk> {
        if self.common().contiguous {
            if let Some(chunk) = self.chunk_queue.pop() {
                self.total_chunks.fetch_add(1, Ordering::SeqCst);
                return Some(chunk);
            }
        }
        let start = self
            .common()
            .grow_discontiguous_space(space.common().descriptor, 1, None);
        if start.is_zero() {
            return None;
        }
        if let Err(mmap_error) = crate::mmtk::MMAPPER
            .ensure_mapped(
                start,
                PAGES_IN_CHUNK as _,
                MmapStrategy::INTERNAL_MEMORY,
                &memory::MmapAnnotation::Space {
                    name: space.get_name(),
                },
            )
            .and(self.common().metadata.try_map_metadata_space(
                start,
                BYTES_IN_CHUNK,
                space.get_name(),
            ))
        {
            crate::util::memory::handle_mmap_error::<VM>(mmap_error, VMThread::UNINITIALIZED);
        }
        space.grow_space(start, BYTES_IN_CHUNK, true);
        self.total_chunks.fetch_add(1, Ordering::SeqCst);
        CHUNK_LIVE_BLOCKS.store_atomic(start, 0u16, Ordering::SeqCst);
        Some(Chunk::from_aligned_address(start))
    }

    fn free_chunk(&self, chunk: Chunk) {
        self.total_chunks.fetch_sub(1, Ordering::SeqCst);
        if self.common().contiguous {
            self.chunk_queue.push(chunk);
        } else {
            self.common().release_discontiguous_chunks(chunk.start());
        }
    }

    pub fn release_block(&self, block: B, single_thread: bool) {
        let blocks =
            CHUNK_LIVE_BLOCKS.fetch_sub_atomic(Chunk::align(block.start()), 1u16, Ordering::SeqCst)
                - 1;
        assert!(blocks <= Self::BLOCKS_IN_CHUNK as u16);
        let pages = 1 << Self::LOG_PAGES;
        self.common().accounting.release(pages as _);
        self.block_alloc.free(self, block, single_thread);
    }

    pub fn flush_all(&self) {}

    pub fn available_pages(&self) -> usize {
        let total = self.total_chunks.load(Ordering::SeqCst)
            << (LOG_BYTES_IN_CHUNK - LOG_BYTES_IN_PAGE as usize);
        total.saturating_sub(self.reserved_pages())
    }
}

pub trait BlockAlloc<VM: VMBinding, B: Region> {
    fn new() -> Self;
    fn alloc(&self, bpr: &BlockPageResource<VM, B>, space: &dyn Space<VM>) -> Option<(B, bool)>;
    fn free(&self, bpr: &BlockPageResource<VM, B>, b: B, single_thread: bool);
}

#[derive(Copy, Clone)]
struct Cursor(u32, u32);

unsafe impl bytemuck::NoUninit for Cursor {}

struct LockFreeListBlockAlloc<B: Region> {
    cursor: Atomic<Cursor>,
    head: Atomic<Address>,
    _p: PhantomData<B>,
}

impl<B: Region> LockFreeListBlockAlloc<B> {
    const LOG_BLOCKS_IN_CHUNK: usize = Chunk::LOG_BYTES - B::LOG_BYTES;
    const BLOCKS_IN_CHUNK: usize = 1 << Self::LOG_BLOCKS_IN_CHUNK;
    const SYNC: bool = false;

    fn alloc_fast(&self) -> Option<B> {
        if Self::SYNC {
            // 1. bump the cursor
            let Cursor(c, b) = self.cursor.load(Ordering::Relaxed);
            if b < Self::BLOCKS_IN_CHUNK as u32 {
                self.cursor.store(Cursor(c, b + 1), Ordering::Relaxed);
                let c = crate::util::conversions::chunk_index_to_address(c as usize);
                return Some(B::from_aligned_address(c + ((b as usize) << B::LOG_BYTES)));
            }
            // 2. pop from list
            let top = self.head.load(Ordering::Relaxed);
            if top.is_zero() {
                return None;
            }
            let new_top = unsafe { top.load::<Address>() };
            self.head.store(new_top, Ordering::Relaxed);
            return Some(B::from_aligned_address(top));
        }
        if cfg!(feature = "block_alloc_order_1") {
            // 2. pop from list
            loop {
                std::hint::spin_loop();
                let top = self.head.load(Ordering::Relaxed);
                if top.is_zero() {
                    break;
                }
                let new_top = unsafe { top.load::<Address>() };
                if self
                    .head
                    .compare_exchange(top, new_top, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    return Some(B::from_aligned_address(top));
                }
            }
        }
        // 1. bump the cursor
        if self.cursor.load(Ordering::Relaxed).1 < Self::BLOCKS_IN_CHUNK as u32 {
            let result =
                self.cursor
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |Cursor(c, b)| {
                        if b < Self::BLOCKS_IN_CHUNK as u32 {
                            Some(Cursor(c, b + 1))
                        } else {
                            None
                        }
                    });
            if let Ok(Cursor(c, b)) = result {
                let c = crate::util::conversions::chunk_index_to_address(c as usize);
                return Some(B::from_aligned_address(c + ((b as usize) << B::LOG_BYTES)));
            }
        }
        if !cfg!(feature = "block_alloc_order_1") {
            // 2. pop from list
            loop {
                std::hint::spin_loop();
                let top = self.head.load(Ordering::Relaxed);
                if top.is_zero() {
                    return None;
                }
                let new_top = unsafe { top.load::<Address>() };
                if self
                    .head
                    .compare_exchange(top, new_top, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    return Some(B::from_aligned_address(top));
                }
            }
        }
        None
    }

    fn add_chunk(&self, c: Chunk) -> B {
        let new_cursor = c.start() + B::BYTES;
        let chunk_index = new_cursor.chunk_index() as u32;
        self.cursor.store(Cursor(chunk_index, 1), Ordering::Relaxed);
        B::from_aligned_address(c.start())
    }
}

impl<VM: VMBinding, B: Region> BlockAlloc<VM, B> for LockFreeListBlockAlloc<B> {
    fn new() -> Self {
        Self {
            head: Atomic::new(Address::ZERO),
            cursor: Atomic::new(Cursor(0, Self::BLOCKS_IN_CHUNK as _)),
            _p: PhantomData,
        }
    }

    fn alloc(&self, bpr: &BlockPageResource<VM, B>, space: &dyn Space<VM>) -> Option<(B, bool)> {
        if !Self::SYNC {
            if let Some(b) = self.alloc_fast() {
                return Some((b, false));
            }
        }
        let _sync = bpr.sync.lock().unwrap();
        if let Some(b) = self.alloc_fast() {
            return Some((b, false));
        }
        if let Some(chunk) = bpr.alloc_chunk(space) {
            let block = self.add_chunk(chunk);
            return Some((block, true));
        }
        return None;
    }

    fn free(&self, bpr: &BlockPageResource<VM, B>, b: B, single_thread: bool) {
        if single_thread {
            let old_top = self.head.load(Ordering::Relaxed);
            unsafe {
                b.start().store(old_top);
            }
            self.head.store(b.start(), Ordering::Relaxed);
            return;
        }
        if Self::SYNC {
            let _sync = bpr.sync.lock().unwrap();
            let old_top = self.head.load(Ordering::Relaxed);
            unsafe { b.start().store(old_top) };
            self.head.store(b.start(), Ordering::Relaxed);
            return;
        }
        loop {
            std::hint::spin_loop();
            let old_top = self.head.load(Ordering::Relaxed);
            unsafe { b.start().store(old_top) };
            if self
                .head
                .compare_exchange(old_top, b.start(), Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }
}
