use super::pageresource::{PRAllocFail, PRAllocResult};
use super::PageResource;
use crate::util::address::Address;
use crate::util::constants::*;
use crate::util::conversions::bytes_to_pages;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::opaque_pointer::*;
use crate::vm::*;
use atomic::{Atomic, Ordering};
use spin::RwLock;
use std::cell::UnsafeCell;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const UNINITIALIZED_WATER_MARK: i32 = -1;
const LOCAL_BUFFER_SIZE: usize = 128;

/// A fast PageResource for fixed-size block allocation only.
pub struct BlockPageResource<VM: VMBinding> {
    common: CommonPageResource,
    /// Block granularity
    log_pages: usize,
    /// A buffer for storing all the free blocks
    block_queue: BlockPool<Address>,
    /// Top address of the allocated contiguous space
    highwater: Atomic<Address>,
    /// Limit of the contiguous space
    limit: Address,
    /// Slow-path allocation synchronization
    sync: Mutex<()>,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> PageResource<VM> for BlockPageResource<VM> {
    #[inline(always)]
    fn common(&self) -> &CommonPageResource {
        &self.common
    }

    #[inline(always)]
    fn common_mut(&mut self) -> &mut CommonPageResource {
        &mut self.common
    }

    #[inline]
    fn alloc_pages(
        &self,
        _space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        self.alloc_pages_fast(reserved_pages, required_pages, tls)
    }

    fn get_available_physical_pages(&self) -> usize {
        debug_assert!(self.common.contiguous);
        let _sync = self.sync.lock().unwrap();
        bytes_to_pages(self.limit - self.highwater.load(Ordering::SeqCst))
    }
}

impl<VM: VMBinding> BlockPageResource<VM> {
    pub fn new_contiguous(
        log_pages: usize,
        start: Address,
        bytes: usize,
        vm_map: &'static VMMap,
        num_workers: usize,
    ) -> Self {
        let growable = cfg!(target_pointer_width = "64");
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self {
            log_pages,
            common: CommonPageResource::new(true, growable, vm_map),
            // Highwater starts from the start address of the contiguous space
            highwater: Atomic::new(start),
            limit: (start + bytes).align_up(BYTES_IN_CHUNK),
            block_queue: BlockPool::new(num_workers),
            sync: Mutex::new(()),
            _p: PhantomData,
        }
    }

    /// Grow contiguous space
    #[cold]
    fn alloc_pages_slow_sync(
        &self,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        let _guard = self.sync.lock().unwrap();
        // Retry fast allocation
        if let Some(block) = self.block_queue.pop() {
            self.commit_pages(reserved_pages, required_pages, tls);
            return Result::Ok(PRAllocResult {
                start: block,
                pages: required_pages,
                new_chunk: false,
            });
        }
        // Grow space (a chunk at a time)
        // 1. Raise highwater by chunk size
        let start: Address =
            match self
                .highwater
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| {
                    if x >= self.limit {
                        None
                    } else {
                        Some(x + BYTES_IN_CHUNK)
                    }
                }) {
                Ok(a) => a,
                _ => return Result::Err(PRAllocFail),
            };
        assert!(start.is_aligned_to(BYTES_IN_CHUNK));
        // 2. Take the first block int the chunk as the allocation result
        let first_block = start;
        // 3. Push all remaining blocks to a block list
        let last_block = start + BYTES_IN_CHUNK;
        let block_size = 1usize << (self.log_pages + LOG_BYTES_IN_PAGE as usize);
        let array = BlockQueue::new();
        let mut cursor = start + block_size;
        while cursor < last_block {
            unsafe { array.push_relaxed(cursor).unwrap() };
            cursor += block_size;
        }
        // 4. Push the block list to the global pool
        self.block_queue.add_global_array(array);
        // Finish slow-allocation
        self.commit_pages(reserved_pages, required_pages, tls);
        Result::Ok(PRAllocResult {
            start: first_block,
            pages: required_pages,
            new_chunk: true,
        })
    }

    /// Allocate a block
    #[inline(always)]
    fn alloc_pages_fast(
        &self,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        debug_assert_eq!(reserved_pages, required_pages);
        debug_assert_eq!(reserved_pages, 1 << self.log_pages);
        // Fast allocate from the blocks list
        if let Some(block) = self.block_queue.pop() {
            self.commit_pages(reserved_pages, required_pages, tls);
            return Result::Ok(PRAllocResult {
                start: block,
                pages: required_pages,
                new_chunk: false,
            });
        }
        // Slow-path：we need to grow space
        self.alloc_pages_slow_sync(reserved_pages, required_pages, tls)
    }

    #[inline]
    pub fn release_pages(&self, first: Address) {
        debug_assert!(self.common.contiguous);
        debug_assert!(first.is_aligned_to(1usize << (self.log_pages + LOG_BYTES_IN_PAGE as usize)));
        let pages = 1 << self.log_pages;
        debug_assert!(pages as usize <= self.common.accounting.get_committed_pages());
        self.common.accounting.release(pages as _);
        self.block_queue.push(first)
    }

    pub fn flush_all(&self) {
        self.block_queue.flush_all()
    }
}

/// A block list that supports fast lock-free push/pop operations
struct BlockQueue<Block> {
    cursor: AtomicUsize,
    data: UnsafeCell<Vec<Block>>,
}

impl<Block: Copy + Default> BlockQueue<Block> {
    const CAPACITY: usize = 256;

    /// Create an array
    #[inline(always)]
    fn new() -> Self {
        let default_block = Default::default();
        Self {
            cursor: AtomicUsize::new(0),
            data: UnsafeCell::new(vec![default_block; Self::CAPACITY]),
        }
    }

    /// Get an entry
    #[inline(always)]
    fn get_entry(&self, i: usize) -> Block {
        unsafe { (*self.data.get())[i] }
    }

    /// Set an entry.
    ///
    /// It's unsafe unless the array is accessed by only one thread (i.e. used as a thread-local array).
    #[inline(always)]
    unsafe fn set_entry(&self, i: usize, block: Block) {
        (*self.data.get())[i] = block
    }

    /// Non-atomically push an element.
    ///
    /// It's unsafe unless the array is accessed by only one thread (i.e. used as a thread-local array).
    #[inline(always)]
    unsafe fn push_relaxed(&self, block: Block) -> Result<(), Block> {
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
    #[inline(always)]
    fn pop(&self) -> Option<Block> {
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
    #[inline(always)]
    fn len(&self) -> usize {
        self.cursor.load(Ordering::SeqCst)
    }

    /// Test if the array is empty
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate all elements in the array
    #[inline]
    fn iterate_blocks(&self, f: &mut impl FnMut(Block)) {
        let len = self.len();
        for i in 0..len {
            f(self.get_entry(i))
        }
    }

    /// Replace the array with a new array.
    ///
    /// Return the old array
    #[inline(always)]
    fn replace(&self, new_array: Self) -> Self {
        // Swap cursor
        let temp = self.cursor.load(Ordering::Relaxed);
        self.cursor
            .store(new_array.cursor.load(Ordering::Relaxed), Ordering::Relaxed);
        new_array.cursor.store(temp, Ordering::Relaxed);
        // Swap data
        unsafe {
            std::mem::swap(&mut *self.data.get(), &mut *new_array.data.get());
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
pub struct BlockPool<Block> {
    /// First global BlockArray for fast allocation
    head_global_freed_blocks: RwLock<Option<BlockQueue<Block>>>,
    /// A list of BlockArray that is flushed to the global pool
    global_freed_blocks: RwLock<Vec<BlockQueue<Block>>>,
    /// Thread-local block queues
    worker_local_freed_blocks: Vec<BlockQueue<Block>>,
    /// Total number of blocks in the whole BlockQueue
    count: AtomicUsize,
}

impl<Block: Debug + Copy + Default> BlockPool<Block> {
    /// Create a BlockQueue
    pub fn new(num_workers: usize) -> Self {
        Self {
            head_global_freed_blocks: Default::default(),
            global_freed_blocks: Default::default(),
            worker_local_freed_blocks: (0..num_workers).map(|_| BlockQueue::new()).collect(),
            count: AtomicUsize::new(0),
        }
    }

    /// Add a BlockArray to the global pool
    fn add_global_array(&self, array: BlockQueue<Block>) {
        self.count.fetch_add(array.len(), Ordering::SeqCst);
        self.global_freed_blocks.write().push(array);
    }

    /// Push a block to the thread-local queue
    #[inline(always)]
    pub fn push(&self, block: Block) {
        self.count.fetch_add(1, Ordering::SeqCst);
        let id = crate::scheduler::current_worker_ordinal().unwrap();
        let failed = unsafe {
            self.worker_local_freed_blocks[id]
                .push_relaxed(block)
                .is_err()
        };
        if failed {
            let queue = BlockQueue::new();
            unsafe { queue.push_relaxed(block).unwrap() };
            let old_queue = self.worker_local_freed_blocks[id].replace(queue);
            assert!(!old_queue.is_empty());
            self.global_freed_blocks.write().push(old_queue);
        }
    }

    /// Pop a block from the global pool
    #[inline(always)]
    pub fn pop(&self) -> Option<Block> {
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
    #[inline(always)]
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
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Iterate all the blocks in the BlockQueue
    #[inline]
    pub fn iterate_blocks(&self, f: &mut impl FnMut(Block)) {
        for array in &*self.head_global_freed_blocks.read() {
            array.iterate_blocks(f)
        }
        for array in &*self.global_freed_blocks.read() {
            array.iterate_blocks(f);
        }
        for array in &self.worker_local_freed_blocks {
            array.iterate_blocks(f);
        }
    }

    pub fn reset(&self) {}
}
