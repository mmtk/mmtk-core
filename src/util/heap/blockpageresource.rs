use super::chunk_map::Chunk;
use super::pageresource::{PRAllocFail, PRAllocResult};
use super::{FreeListPageResource, PageResource};
use crate::util::address::Address;
use crate::util::constants::*;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::layout::VMMap;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::linear_scan::Region;
use crate::util::opaque_pointer::*;
use crate::util::rust_util::zeroed_alloc::new_zeroed_vec;
use crate::vm::*;
use atomic::{Atomic, Ordering};
use crossbeam::queue::SegQueue;
use spin::RwLock;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
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
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        if let Some((block, new_chunk)) = self.block_alloc.alloc(self, space_descriptor) {
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
}

impl<VM: VMBinding, B: Region> BlockPageResource<VM, B> {
    /// Block granularity in pages
    const LOG_PAGES: usize = B::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    const BLOCKS_IN_CHUNK: usize = 1 << (Chunk::LOG_BYTES - B::LOG_BYTES);

    pub fn new_contiguous(
        log_pages: usize,
        start: Address,
        bytes: usize,
        vm_map: &'static dyn VMMap,
        _num_workers: usize,
    ) -> Self {
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self {
            flpr: FreeListPageResource::new_contiguous(start, bytes, vm_map),
            block_alloc: <LockFreeListBlockAlloc<B> as BlockAlloc<VM, B>>::new(),
            sync: Mutex::default(),
            chunk_queue: SegQueue::new(),
            total_chunks: AtomicUsize::new(0),
            _p: PhantomData,
        }
    }

    pub fn new_discontiguous(
        log_pages: usize,
        vm_map: &'static dyn VMMap,
        _num_workers: usize,
    ) -> Self {
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self {
            flpr: FreeListPageResource::new_discontiguous(vm_map),
            block_alloc: <LockFreeListBlockAlloc<B> as BlockAlloc<VM, B>>::new(),
            sync: Mutex::default(),
            chunk_queue: SegQueue::new(),
            total_chunks: AtomicUsize::new(0),
            _p: PhantomData,
        }
    }

    fn alloc_chunk(&self, space_descriptor: SpaceDescriptor) -> Option<Chunk> {
        if self.common().contiguous {
            if let Some(chunk) = self.chunk_queue.pop() {
                self.total_chunks.fetch_add(1, Ordering::SeqCst);
                return Some(chunk);
            }
        }
        let start = self
            .common()
            .grow_discontiguous_space(space_descriptor, 1, None);
        if start.is_zero() {
            return None;
        }
        self.total_chunks.fetch_add(1, Ordering::SeqCst);
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
    fn alloc(
        &self,
        bpr: &BlockPageResource<VM, B>,
        space_descriptor: SpaceDescriptor,
    ) -> Option<(B, bool)>;
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

    fn alloc(
        &self,
        bpr: &BlockPageResource<VM, B>,
        space_descriptor: SpaceDescriptor,
    ) -> Option<(B, bool)> {
        if !Self::SYNC {
            if let Some(b) = self.alloc_fast() {
                return Some((b, false));
            }
        }
        let _sync = bpr.sync.lock().unwrap();
        if let Some(b) = self.alloc_fast() {
            return Some((b, false));
        }
        if let Some(chunk) = bpr.alloc_chunk(space_descriptor) {
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

/// A block list that supports fast lock-free push/pop operations
struct BlockQueue<B: Region> {
    /// The number of elements in the queue.
    cursor: AtomicUsize,
    /// The underlying data storage.
    ///
    /// -   `UnsafeCell<T>`: It may be accessed by multiple threads.
    /// -   `Box<[T]>`: It holds an array allocated on the heap.  It cannot be resized, but can be
    ///     replaced with another array as a whole.
    /// -   `MaybeUninit<T>`: It may contain uninitialized elements.
    ///
    /// The implementaiton of `BlockQueue` must ensure there is no data race, and it never reads
    /// uninitialized elements.
    data: UnsafeCell<Box<[MaybeUninit<B>]>>,
}

impl<B: Region> BlockQueue<B> {
    /// Create an array
    fn new() -> Self {
        let zeroed_vec = new_zeroed_vec(Self::CAPACITY);
        let boxed_slice = zeroed_vec.into_boxed_slice();
        let data = UnsafeCell::new(boxed_slice);
        Self {
            cursor: AtomicUsize::new(0),
            data,
        }
    }
}

impl<B: Region> BlockQueue<B> {
    const CAPACITY: usize = 256;

    /// Get an entry
    fn get_entry(&self, i: usize) -> B {
        unsafe { (*self.data.get())[i].assume_init() }
    }

    /// Set an entry.
    ///
    /// It's unsafe unless the array is accessed by only one thread (i.e. used as a thread-local array).
    unsafe fn set_entry(&self, i: usize, block: B) {
        (*self.data.get())[i].write(block);
    }

    /// Non-atomically push an element.
    ///
    /// It's unsafe unless the array is accessed by only one thread (i.e. used as a thread-local array).
    unsafe fn push_relaxed(&self, block: B) -> Result<(), B> {
        let i = self.cursor.load(Ordering::Relaxed);
        if i < Self::CAPACITY {
            self.set_entry(i, block);
            self.cursor.store(i + 1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(block)
        }
    }

    /// Atomically pop an element from the array.
    fn pop(&self) -> Option<B> {
        let i = self
            .cursor
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |i| {
                if i > 0 {
                    Some(i - 1)
                } else {
                    None
                }
            });
        if let Ok(i) = i {
            Some(self.get_entry(i - 1))
        } else {
            None
        }
    }

    /// Get array size
    fn len(&self) -> usize {
        self.cursor.load(Ordering::SeqCst)
    }

    /// Test if the array is empty
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate all elements in the array
    fn iterate_blocks(&self, f: &mut impl FnMut(B)) {
        let len = self.len();
        for i in 0..len {
            f(self.get_entry(i))
        }
    }

    /// Replace the array with a new array.
    ///
    /// Return the old array
    fn replace(&self, new_array: Self) -> Self {
        // Swap cursor
        let temp = self.cursor.load(Ordering::Relaxed);
        self.cursor
            .store(new_array.cursor.load(Ordering::Relaxed), Ordering::Relaxed);
        new_array.cursor.store(temp, Ordering::Relaxed);
        // Swap data
        unsafe {
            core::ptr::swap(self.data.get(), new_array.data.get());
        }
        // Return old array
        new_array
    }
}

/// A block queue which contains a global pool and a set of thread-local queues.
///
/// Mutator or collector threads always allocate blocks by poping from the global pool。
///
/// Collector threads free blocks to their thread-local queues, and then flush to the global pools before GC ends.
pub struct BlockPool<B: Region> {
    /// First global BlockArray for fast allocation
    head_global_freed_blocks: RwLock<Option<BlockQueue<B>>>,
    /// A list of BlockArray that is flushed to the global pool
    global_freed_blocks: RwLock<Vec<BlockQueue<B>>>,
    /// Thread-local block queues
    worker_local_freed_blocks: Vec<BlockQueue<B>>,
    /// Total number of blocks in the whole BlockQueue
    count: AtomicUsize,
}

impl<B: Region> BlockPool<B> {
    /// Create a BlockQueue
    pub fn new(num_workers: usize) -> Self {
        Self {
            head_global_freed_blocks: RwLock::new(None),
            global_freed_blocks: RwLock::new(vec![]),
            worker_local_freed_blocks: (0..num_workers).map(|_| BlockQueue::new()).collect(),
            count: AtomicUsize::new(0),
        }
    }

    /// Add a BlockArray to the global pool
    fn add_global_array(&self, array: BlockQueue<B>) {
        self.count.fetch_add(array.len(), Ordering::SeqCst);
        self.global_freed_blocks.write().push(array);
    }

    /// Push a block to the thread-local queue
    pub fn push(&self, block: B) {
        self.count.fetch_add(1, Ordering::SeqCst);
        let id = crate::scheduler::current_worker_ordinal().unwrap();
        let failed = unsafe {
            self.worker_local_freed_blocks[id]
                .push_relaxed(block)
                .is_err()
        };
        if failed {
            let queue = BlockQueue::new();
            let result = unsafe { queue.push_relaxed(block) };
            debug_assert!(result.is_ok());
            let old_queue = self.worker_local_freed_blocks[id].replace(queue);
            assert!(!old_queue.is_empty());
            self.global_freed_blocks.write().push(old_queue);
        }
    }

    /// Pop a block from the global pool
    pub fn pop(&self) -> Option<B> {
        if self.len() == 0 {
            return None;
        }
        let head_global_freed_blocks = self.head_global_freed_blocks.upgradeable_read();
        if let Some(block) = head_global_freed_blocks.as_ref().and_then(|q| q.pop()) {
            self.count.fetch_sub(1, Ordering::SeqCst);
            Some(block)
        } else {
            let mut global_freed_blocks = self.global_freed_blocks.write();
            // Retry fast-alloc
            if let Some(block) = head_global_freed_blocks.as_ref().and_then(|q| q.pop()) {
                self.count.fetch_sub(1, Ordering::SeqCst);
                return Some(block);
            }
            // Get a new list of blocks for allocation
            let blocks = global_freed_blocks.pop()?;
            let block = blocks.pop().unwrap();
            if !blocks.is_empty() {
                let mut head_global_freed_blocks = head_global_freed_blocks.upgrade();
                debug_assert!(head_global_freed_blocks
                    .as_ref()
                    .map(|blocks| blocks.is_empty())
                    .unwrap_or(true));
                *head_global_freed_blocks = Some(blocks);
            }
            self.count.fetch_sub(1, Ordering::SeqCst);
            Some(block)
        }
    }

    /// Flush a given thread-local queue to the global pool
    fn flush(&self, id: usize) {
        if !self.worker_local_freed_blocks[id].is_empty() {
            let queue = self.worker_local_freed_blocks[id].replace(BlockQueue::new());
            if !queue.is_empty() {
                self.global_freed_blocks.write().push(queue)
            }
        }
    }

    /// Flush all thread-local queues to the global pool
    pub fn flush_all(&self) {
        if self.len() == 0 {
            return;
        }
        for i in 0..self.worker_local_freed_blocks.len() {
            self.flush(i)
        }
    }

    /// Get total number of blocks in the whole BlockQueue
    pub fn len(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Iterate all the blocks in the BlockQueue
    pub fn iterate_blocks(&self, f: &mut impl FnMut(B)) {
        if let Some(array) = &*self.head_global_freed_blocks.read() {
            array.iterate_blocks(f);
        }
        for array in &*self.global_freed_blocks.read() {
            array.iterate_blocks(f);
        }
        for array in &self.worker_local_freed_blocks {
            array.iterate_blocks(f);
        }
    }
}
