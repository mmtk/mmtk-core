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
use crate::vm::*;
use atomic::Ordering;
use spin::RwLock;
use std::cell::UnsafeCell;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const UNINITIALIZED_WATER_MARK: i32 = -1;
const LOCAL_BUFFER_SIZE: usize = 128;

/// A fast PageResource for fixed-size block allocation only.
pub struct BlockPageResource<VM: VMBinding, B: Region + 'static> {
    flpr: FreeListPageResource<VM>,
    /// A buffer for storing all the free blocks
    block_queue: BlockPool<B>,
    /// Slow-path allocation synchronization
    sync: Mutex<()>,
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
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        self.alloc_pages_fast(space_descriptor, reserved_pages, required_pages, tls)
    }

    fn get_available_physical_pages(&self) -> usize {
        let _sync = self.sync.lock().unwrap();
        self.flpr.get_available_physical_pages()
    }
}

impl<VM: VMBinding, B: Region> BlockPageResource<VM, B> {
    /// Block granularity in pages
    const LOG_PAGES: usize = B::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;

    pub fn new_contiguous(
        log_pages: usize,
        start: Address,
        bytes: usize,
        vm_map: &'static dyn VMMap,
        num_workers: usize,
    ) -> Self {
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self {
            flpr: FreeListPageResource::new_contiguous(start, bytes, vm_map),
            block_queue: BlockPool::new(num_workers),
            sync: Mutex::new(()),
        }
    }

    pub fn new_discontiguous(
        log_pages: usize,
        vm_map: &'static dyn VMMap,
        num_workers: usize,
    ) -> Self {
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self {
            flpr: FreeListPageResource::new_discontiguous(vm_map),
            block_queue: BlockPool::new(num_workers),
            sync: Mutex::new(()),
        }
    }

    /// Grow contiguous space
    fn alloc_pages_slow_sync(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        let _guard = self.sync.lock().unwrap();
        // Retry fast allocation
        if let Some(block) = self.block_queue.pop() {
            self.commit_pages(reserved_pages, required_pages, tls);
            return Result::Ok(PRAllocResult {
                start: block.start(),
                pages: required_pages,
                new_chunk: false,
            });
        }
        // Grow space (a chunk at a time)
        // 1. Grow space
        let start: Address = match self.flpr.allocate_one_chunk_no_commit(space_descriptor) {
            Ok(result) => result.start,
            err => return err,
        };
        assert!(start.is_aligned_to(BYTES_IN_CHUNK));
        // 2. Take the first block int the chunk as the allocation result
        let first_block = start;
        // 3. Push all remaining blocks to one or more block lists
        let last_block = start + BYTES_IN_CHUNK;
        let mut array = BlockQueue::new();
        let mut cursor = start + B::BYTES;
        while cursor < last_block {
            let result = unsafe { array.push_relaxed(B::from_aligned_address(cursor)) };
            if let Err(block) = result {
                self.block_queue.add_global_array(array);
                array = BlockQueue::new();
                let result2 = unsafe { array.push_relaxed(block) };
                debug_assert!(result2.is_ok());
            }
            cursor += B::BYTES;
        }
        debug_assert!(!array.is_empty());
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
    fn alloc_pages_fast(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        debug_assert_eq!(reserved_pages, required_pages);
        debug_assert_eq!(reserved_pages, 1 << Self::LOG_PAGES);
        // Fast allocate from the blocks list
        if let Some(block) = self.block_queue.pop() {
            self.commit_pages(reserved_pages, required_pages, tls);
            return Result::Ok(PRAllocResult {
                start: block.start(),
                pages: required_pages,
                new_chunk: false,
            });
        }
        // Slow-path：we need to grow space
        self.alloc_pages_slow_sync(space_descriptor, reserved_pages, required_pages, tls)
    }

    pub fn release_block(&self, block: B) {
        let pages = 1 << Self::LOG_PAGES;
        debug_assert!(pages as usize <= self.common().accounting.get_committed_pages());
        self.common().accounting.release(pages as _);
        self.block_queue.push(block)
    }

    pub fn flush_all(&self) {
        self.block_queue.flush_all()
        // TODO: For 32-bit space, we may want to free some contiguous chunks.
    }
}

/// A block list that supports fast lock-free push/pop operations
struct BlockQueue<B: Region> {
    cursor: AtomicUsize,
    data: UnsafeCell<Vec<B>>,
}

impl<B: Region> BlockQueue<B> {
    /// Create an array
    fn new() -> Self {
        let default_block = B::from_aligned_address(Address::ZERO);
        Self {
            cursor: AtomicUsize::new(0),
            data: UnsafeCell::new(vec![default_block; Self::CAPACITY]),
        }
    }
}

impl<B: Region> BlockQueue<B> {
    const CAPACITY: usize = 256;

    /// Get an entry
    fn get_entry(&self, i: usize) -> B {
        unsafe { (*self.data.get())[i] }
    }

    /// Set an entry.
    ///
    /// It's unsafe unless the array is accessed by only one thread (i.e. used as a thread-local array).
    unsafe fn set_entry(&self, i: usize, block: B) {
        (*self.data.get())[i] = block
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
        let id = crate::scheduler::current_worker_ordinal();
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
}
