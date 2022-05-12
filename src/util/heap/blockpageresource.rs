use super::pageresource::{PRAllocFail, PRAllocResult};
use super::PageResource;
use crate::util::address::Address;
use crate::util::constants::*;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::opaque_pointer::*;
use crate::vm::*;
use atomic::{Atomic, Ordering};
use spin::rwlock::RwLock;
use std::cell::UnsafeCell;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const UNINITIALIZED_WATER_MARK: i32 = -1;
const LOCAL_BUFFER_SIZE: usize = 128;

pub struct BlockPageResource<VM: VMBinding> {
    common: CommonPageResource,
    log_pages: usize,
    block_queue: BlockQueue<Address>,
    highwater: Atomic<Address>,
    sync: Mutex<()>,
    limit: Address,
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

    #[inline(always)]
    fn adjust_for_metadata(&self, pages: usize) -> usize {
        pages
    }
}

impl<VM: VMBinding> BlockPageResource<VM> {
    pub fn new_contiguous(
        log_pages: usize,
        start: Address,
        bytes: usize,
        vm_map: &'static VMMap,
    ) -> Self {
        let growable = cfg!(target_pointer_width = "64");
        assert!((1 << log_pages) <= PAGES_IN_CHUNK);
        Self {
            log_pages,
            common: CommonPageResource::new(true, growable, vm_map),
            highwater: Atomic::new(start),
            limit: (start + bytes).align_up(BYTES_IN_CHUNK),
            block_queue: BlockQueue::new(),
            sync: Mutex::new(()),
            _p: PhantomData,
        }
    }

    pub fn init(&mut self, num_workers: usize) {
        self.block_queue.init(num_workers);
    }

    /// The caller needs to ensure this is called by only one thread.
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
        // Grow space
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
        let first_block = start;
        let last_block = start + BYTES_IN_CHUNK;
        let block_size = 1usize << (self.log_pages + LOG_BYTES_IN_PAGE as usize);
        let array = BlockArray::new();
        let mut cursor = start + block_size;
        while cursor < last_block {
            array.push_relaxed(cursor).unwrap();
            cursor += block_size;
        }
        self.block_queue.add_global_array(array);
        self.commit_pages(reserved_pages, required_pages, tls);
        Result::Ok(PRAllocResult {
            start: first_block,
            pages: PAGES_IN_CHUNK,
            new_chunk: true,
        })
    }

    /// The caller needs to ensure this is called by only one thread.
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

struct BlockArray<Block> {
    cursor: AtomicUsize,
    data: UnsafeCell<Vec<Block>>,
    capacity: usize,
}

impl<Block: Copy> BlockArray<Block> {
    const LOCAL_BUFFER_SIZE: usize = 256;

    #[inline(always)]
    fn new() -> Self {
        let mut array = Self {
            cursor: AtomicUsize::new(0),
            data: UnsafeCell::new(Vec::with_capacity(Self::LOCAL_BUFFER_SIZE)),
            capacity: Self::LOCAL_BUFFER_SIZE,
        };
        unsafe { array.data.get_mut().set_len(Self::LOCAL_BUFFER_SIZE) }
        array
    }

    #[inline(always)]
    fn get_entry(&self, i: usize) -> Block {
        unsafe { (*self.data.get())[i] }
    }

    #[inline(always)]
    unsafe fn set_entry(&self, i: usize, block: Block) {
        (*self.data.get())[i] = block
    }

    #[inline(always)]
    fn push_relaxed(&self, block: Block) -> Result<(), Block> {
        let i = self.cursor.load(Ordering::Relaxed);
        if i < self.capacity {
            unsafe {
                self.set_entry(i, block);
            }
            self.cursor.store(i + 1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(block)
        }
    }

    #[inline(always)]
    fn pop(&self) -> Option<Block> {
        let i = self
            .cursor
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |i| {
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

    #[inline(always)]
    fn len(&self) -> usize {
        self.cursor.load(Ordering::Relaxed)
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    fn iterate_blocks(&self, f: &mut impl FnMut(Block)) {
        let len = self.len();
        for i in 0..len {
            f(self.get_entry(i))
        }
    }

    fn replace(&self, new_array: Self) -> Self {
        assert_eq!(self.capacity, new_array.capacity);
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

pub struct BlockQueue<Block> {
    head_global_freed_blocks: RwLock<Option<BlockArray<Block>>, spin::Yield>,
    global_freed_blocks: RwLock<Vec<BlockArray<Block>>, spin::Yield>,
    worker_local_freed_blocks: Vec<BlockArray<Block>>,
    count: AtomicUsize,
}

impl<Block: Debug + Copy> BlockQueue<Block> {
    pub fn new() -> Self {
        Self {
            head_global_freed_blocks: Default::default(),
            global_freed_blocks: Default::default(),
            worker_local_freed_blocks: vec![],
            count: AtomicUsize::new(0),
        }
    }

    fn init(&mut self, num_workers: usize) {
        let mut worker_local_freed_blocks = vec![];
        worker_local_freed_blocks.resize_with(num_workers, || BlockArray::new());
        self.worker_local_freed_blocks = worker_local_freed_blocks;
    }

    fn add_global_array(&self, array: BlockArray<Block>) {
        self.count.fetch_add(array.len(), Ordering::Relaxed);
        self.global_freed_blocks.write().push(array);
    }

    #[inline(always)]
    pub fn push(&self, block: Block) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let id = crate::scheduler::current_worker_id().unwrap();
        let failed = self.worker_local_freed_blocks[id]
            .push_relaxed(block)
            .is_err();
        if failed {
            let queue = BlockArray::new();
            queue.push_relaxed(block).unwrap();
            let old_queue = self.worker_local_freed_blocks[id].replace(queue);
            if !old_queue.is_empty() {
                self.global_freed_blocks.write().push(old_queue);
            }
        }
    }

    #[inline(always)]
    pub fn pop(&self) -> Option<Block> {
        let head_global_freed_blocks = self.head_global_freed_blocks.upgradeable_read();
        if let Some(block) = head_global_freed_blocks.as_ref().and_then(|q| q.pop()) {
            self.count.fetch_sub(1, Ordering::Relaxed);
            Some(block)
        } else if let Some(blocks) = self.global_freed_blocks.write().pop() {
            let block = blocks.pop().unwrap();
            if !blocks.is_empty() {
                let mut head_global_freed_blocks = head_global_freed_blocks.upgrade();
                *head_global_freed_blocks = Some(blocks);
            }
            self.count.fetch_sub(1, Ordering::Relaxed);
            Some(block)
        } else {
            None
        }
    }

    pub fn flush(&self, id: usize) {
        if !self.worker_local_freed_blocks[id].is_empty() {
            let queue = self.worker_local_freed_blocks[id].replace(BlockArray::new());
            if !queue.is_empty() {
                self.global_freed_blocks.write().push(queue)
            }
        }
    }

    pub fn flush_all(&self) {
        for i in 0..self.worker_local_freed_blocks.len() {
            self.flush(i)
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

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
}
