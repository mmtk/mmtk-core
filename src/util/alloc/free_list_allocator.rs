use std::sync::atomic::AtomicBool;
use std::mem::size_of;

use atomic::Ordering;
use atomic_traits::Atomic;
use crate::policy::marksweepspace::block::Block;
use crate::policy::marksweepspace::metadata::is_marked;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::Space;
use crate::util::alloc_bit::{is_alloced, set_alloc_bit, unset_alloc_bit_unsafe};
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::alloc::Allocator;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;
use crate::Plan;

const MI_BIN_HUGE: usize = 73;
const MI_INTPTR_SHIFT: usize = 3;
const MI_INTPTR_SIZE: usize = 1 << MI_INTPTR_SHIFT;
pub const MI_LARGE_OBJ_SIZE_MAX: usize = 1 << 21;
const MI_LARGE_OBJ_WSIZE_MAX: usize = MI_LARGE_OBJ_SIZE_MAX / MI_INTPTR_SIZE;
const MI_INTPTR_BITS: usize = MI_INTPTR_SIZE * 8;
const MI_BIN_FULL: usize = MI_BIN_HUGE + 1;
const ZERO_BLOCK: Block = Block::from(unsafe { Address::zero() });

// mimalloc init.c:46
pub(crate) const BLOCK_LISTS_EMPTY: [BlockList; MI_BIN_HUGE + 1] = [
    BlockList::new(1 * 4),
    BlockList::new(1 * 4),
    BlockList::new(2 * 4),
    BlockList::new(3 * 4),
    BlockList::new(4 * 4),
    BlockList::new(5 * 4),
    BlockList::new(6 * 4),
    BlockList::new(7 * 4),
    BlockList::new(8 * 4), /* 8 */
    BlockList::new(10 * 4),
    BlockList::new(12 * 4),
    BlockList::new(14 * 4),
    BlockList::new(16 * 4),
    BlockList::new(20 * 4),
    BlockList::new(24 * 4),
    BlockList::new(28 * 4),
    BlockList::new(32 * 4), /* 16 */
    BlockList::new(40 * 4),
    BlockList::new(48 * 4),
    BlockList::new(56 * 4),
    BlockList::new(64 * 4),
    BlockList::new(80 * 4),
    BlockList::new(96 * 4),
    BlockList::new(112 * 4),
    BlockList::new(128 * 4), /* 24 */
    BlockList::new(160 * 4),
    BlockList::new(192 * 4),
    BlockList::new(224 * 4),
    BlockList::new(256 * 4),
    BlockList::new(320 * 4),
    BlockList::new(384 * 4),
    BlockList::new(448 * 4),
    BlockList::new(512 * 4), /* 32 */
    BlockList::new(640 * 4),
    BlockList::new(768 * 4),
    BlockList::new(896 * 4),
    BlockList::new(1024 * 4),
    BlockList::new(1280 * 4),
    BlockList::new(1536 * 4),
    BlockList::new(1792 * 4),
    BlockList::new(2048 * 4), /* 40 */
    BlockList::new(2560 * 4),
    BlockList::new(3072 * 4),
    BlockList::new(3584 * 4),
    BlockList::new(4096 * 4),
    BlockList::new(5120 * 4),
    BlockList::new(6144 * 4),
    BlockList::new(7168 * 4),
    BlockList::new(8192 * 4), /* 48 */
    BlockList::new(10240 * 4),
    BlockList::new(12288 * 4),
    BlockList::new(14336 * 4),
    BlockList::new(16384 * 4),
    BlockList::new(20480 * 4),
    BlockList::new(24576 * 4),
    BlockList::new(28672 * 4),
    BlockList::new(32768 * 4), /* 56 */
    BlockList::new(40960 * 4),
    BlockList::new(49152 * 4),
    BlockList::new(57344 * 4),
    BlockList::new(65536 * 4),
    BlockList::new(81920 * 4),
    BlockList::new(98304 * 4),
    BlockList::new(114688 * 4),
    BlockList::new(131072 * 4), /* 64 */
    BlockList::new(163840 * 4),
    BlockList::new(196608 * 4),
    BlockList::new(229376 * 4),
    BlockList::new(262144 * 4),
    BlockList::new(327680 * 4),
    BlockList::new(393216 * 4),
    BlockList::new(458752 * 4),
    BlockList::new(524288 * 4), /* 72 */
    BlockList::new(MI_LARGE_OBJ_WSIZE_MAX + 1 /* 655360, Huge queue */),
];

type BlockLists = [BlockList; MI_BIN_HUGE + 1];

#[repr(C)]
pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    available_blocks: BlockLists, // = pages in mimalloc
    available_blocks_stress: BlockLists,
    unswept_blocks: BlockLists,
    consumed_blocks: BlockLists,
}

#[derive(Debug)]
#[repr(C)]
pub struct BlockList {
    pub first: Block,
    pub last: Block,
    size: usize,
    lock: AtomicBool,
}

impl BlockList {
    const fn new(size: usize) -> BlockList {
        BlockList {
            first: ZERO_BLOCK,
            last: ZERO_BLOCK,
            size,
            lock: AtomicBool::new(false),
        }
    }

    fn is_empty(&self) -> bool {
        self.first.is_zero()
    }

    pub fn remove<VM: VMBinding>(&mut self, block: Block) {
        eprintln!("r {} from {:?}, lock={}", block.start(), self as *mut _, self.lock.load(Ordering::SeqCst));
        let prev = block.load_prev_block::<VM>();
        let next = block.load_next_block::<VM>();
        if prev.is_zero() {
            if next.is_zero() {
                self.first = ZERO_BLOCK;
                self.last = ZERO_BLOCK;
            } else {
                next.store_prev_block::<VM>(ZERO_BLOCK);
                self.first = next;
                next.store_block_list::<VM>(self);
            }
        } else {
            if next.is_zero() {
                prev.store_next_block::<VM>(next);
                prev.store_next_block::<VM>(ZERO_BLOCK);
                self.last = prev;
                prev.store_block_list::<VM>(self);
            } else {
                prev.store_next_block::<VM>(next);
                next.store_prev_block::<VM>(prev);
            }
        }
    }

    fn pop<VM: VMBinding>(&mut self) -> Block {
        eprintln!("pop from {:?}, lock={}", self as *mut _, self.lock.load(Ordering::SeqCst));
        let rtn = self.first;
        if rtn.is_zero() {
            return rtn;
        }
        let next = rtn.load_next_block::<VM>();
        if next.is_zero() {
            self.first = ZERO_BLOCK;
            self.last = ZERO_BLOCK;
        } else {
            self.first = next;
            next.store_prev_block::<VM>(ZERO_BLOCK);
            self.first.store_block_list::<VM>(self);
        }
        rtn.store_next_block::<VM>(ZERO_BLOCK);
        rtn.store_prev_block::<VM>(ZERO_BLOCK);
        rtn
    }

    fn push<VM: VMBinding>(&mut self, block: Block) {
        eprintln!("push {} to {:?}, lock={}", block.start(), self as *mut _, self.lock.load(Ordering::SeqCst));
        if self.is_empty() {
            block.store_next_block::<VM>(ZERO_BLOCK);
            block.store_prev_block::<VM>(ZERO_BLOCK);
            self.first = block;
            self.last = block;
        } else {
            block.store_next_block::<VM>(self.first);
            self.first.store_prev_block::<VM>(block);
            block.store_prev_block::<VM>(ZERO_BLOCK);
            self.first = block;
        }
        block.store_block_list::<VM>(self);
    }
    
    fn append<VM: VMBinding>(&mut self, list: &mut BlockList) {
        eprintln!("append {:?}, lock={}, to {:?}, lock={}", list as *mut _, list.lock.load(Ordering::SeqCst), self as *mut _, self.lock.load(Ordering::SeqCst));
        if !list.is_empty() {
            assert!(list.first.load_prev_block::<VM>().is_zero(), "{} -> {}", list.first.load_prev_block::<VM>().start(), list.first.start());
            if self.is_empty() {
                self.first = list.first;
                self.last = list.last;
            } else {
                assert!(self.first.load_prev_block::<VM>().is_zero(), "{} -> {}", self.first.load_prev_block::<VM>().start(), self.first.start());
                self.last.store_next_block::<VM>(list.first);
                list.first.store_prev_block::<VM>(self.last);
                self.last = list.last;
            }
            let mut block = list.first;
            while !block.is_zero() {
                // eprintln!("appending {} to {:?}", block.start(), self as *mut _);
                block.store_block_list::<VM>(self);
                block = block.load_next_block::<VM>();
            }
            list.reset();
        }
    }

    // fn print<VM: VMBinding>(&self) {
    //     if self.is_empty() {
    //         // eprintln!("{{empty list}}");
    //     } else {
    //         let mut b = self.first;
    //         while b != self.last {
    //             eprint!("{} -> ", b.start());
    //             b = b.load_next_block::<VM>();
    //         }
    //         // eprintln!("{}", b.start());
    //     }
    // }

    fn reset(&mut self) {
        eprintln!("reset list {:?}", self as *mut _);
        self.first = ZERO_BLOCK;
        self.last = ZERO_BLOCK;
    }

    pub fn lock(&mut self) {
        eprintln!("try to lock {:?} (lock = {}, first={}, last={}, size={})", self as *const _, self.lock.load(Ordering::SeqCst), self.first.start(), self.last.start(), self.size);
        let mut success = false;
        while !success {
            success = self.lock.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok();
        }
        eprintln!("successsfully lock {:?}", self as *const _);
    }

    pub fn unlock(&mut self) {
        eprintln!("go to unlock {:?}", self as *const _);
        self.lock.store(false, Ordering::SeqCst);
        eprintln!("unlock {:?}, now {}", self as *const _, self.lock.load(Ordering::SeqCst));
    }
}

unsafe impl<VM: VMBinding> Send for FreeListAllocator<VM> {}

impl<VM: VMBinding> Allocator<VM> for FreeListAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_space(&self) -> &'static dyn crate::policy::space::Space<VM> {
        self.space
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc s={}", size);
        // see mi_heap_malloc_small
        assert!(
            size <= Block::BYTES,
            "Alloc request for {} bytes is too big.",
            size
        );
        debug_assert!(align <= VM::MAX_ALIGNMENT);
        debug_assert!(align >= VM::MIN_ALIGNMENT);
        debug_assert!(offset == 0);

        let addr = self.alloc_from_available(size);
        if addr.is_zero() {
            debug_assert!(self.available_blocks[mi_bin(size) as usize].is_empty());
            return self.alloc_slow(size, align, offset)
        }
        addr
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // try to find an existing block with free cells
        let bin = mi_bin(size);
        if !self.available_blocks[bin as usize].is_empty() {
            // we've just had GC, which has made some blocks available
            return self.alloc_from_available(size);
        }

        let block = self.acquire_block_for_size(size);
        if block.is_zero() {
            // gc
            return unsafe {Address::zero()};
        }

        debug_assert!(!block.is_zero());

        // _mi_page_malloc
        let free_list = block.load_free_list::<VM>();
        debug_assert!(!free_list.is_zero());

        // update free list
        let next_cell = unsafe { free_list.load::<Address>() };
        block.store_free_list::<VM>(next_cell);
        debug_assert!(block.load_free_list::<VM>() == next_cell);

        // set allocation bit
        set_alloc_bit(unsafe { free_list.to_object_reference() });
        debug_assert!(is_alloced(unsafe { free_list.to_object_reference() }));

        free_list
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        Block::BYTES
    }

    #[cfg(not(feature = "eager_sweeping"))]
    fn alloc_slow_once_precise_stress(&mut self, size: usize, align: usize, offset: isize, need_poll: bool) -> Address {
        trace!("allow slow precise stress s={}", size);
        let bin = mi_bin(size) as usize;
        if need_poll {
            let block = self.acquire_fresh_block(size, true);
            if block.is_zero() {
                return unsafe { Address::zero() };
            }
            let free_list = block.load_free_list::<VM>();
            block.store_free_list::<VM>(unsafe { free_list.load::<Address>() });
            return free_list;
        }
        let available = self.available_blocks_stress.get_mut(bin as usize).unwrap();
        if !available.is_empty() {
            let mut block = available.first;
    
            while !block.is_zero() {
                let free_list = block.load_free_list::<VM>();
                if !free_list.is_zero() {
                    // update free list
                    let next_cell = unsafe { free_list.load::<Address>() };
                    block.store_free_list::<VM>(next_cell);
                    debug_assert!(block.load_free_list::<VM>() == next_cell);
                    // set allocation bit
                    set_alloc_bit(unsafe { free_list.to_object_reference() });
                    debug_assert!(is_alloced(unsafe { free_list.to_object_reference() }));
    
                    return free_list;
                }
                available.pop::<VM>();
                self.consumed_blocks.get_mut(bin as usize).unwrap().push::<VM>(block);
    
                block = available.first;
    
            }
        }

        let block = self.acquire_block_for_size(size);
        if block.is_zero() {
            return unsafe {Address::zero()};
        }

        let free_list = block.load_free_list::<VM>();
        block.store_free_list::<VM>(unsafe { free_list.load::<Address>() });

        // set allocation bit
        set_alloc_bit(unsafe { free_list.to_object_reference() });

        free_list
    }
    #[cfg(feature = "eager_sweeping")]
    fn alloc_slow_once_precise_stress(&mut self, size: usize, align: usize, offset: isize, need_poll: bool) -> Address {
        let bin = mi_bin(size) as usize;
        let consumed = self.consumed_blocks.get_mut(bin).unwrap();
        let available = self.available_blocks.get_mut(bin).unwrap();
        consumed.append::<VM>(available);
        unsafe { Address::zero() }
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static MarkSweepSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        FreeListAllocator {
            tls,
            space,
            plan,
            available_blocks: BLOCK_LISTS_EMPTY,
            available_blocks_stress: BLOCK_LISTS_EMPTY,
            unswept_blocks: BLOCK_LISTS_EMPTY,
            consumed_blocks: BLOCK_LISTS_EMPTY,
        }
    }

    fn alloc_from_available(&mut self, size: usize) -> Address {
        trace!("alloc from available s={}", size);
        let bin = mi_bin(size);
        debug_assert!(bin <= MI_BIN_HUGE as u8);

        let available_blocks = &mut self.available_blocks[bin as usize];
        debug_assert!(available_blocks.size >= size);

        let mut block = available_blocks.first;

        while !block.is_zero() {
            trace!("try block {}", block.start());
            let free_list = block.load_free_list::<VM>();
            if !free_list.is_zero() {
                trace!("block {} has free list at {}", block.start(), free_list);
                // update free list
                let next_cell = unsafe { free_list.load::<Address>() };
                block.store_free_list::<VM>(next_cell);
                debug_assert!(block.load_free_list::<VM>() == next_cell);
                // set allocation bit
                set_alloc_bit(unsafe { free_list.to_object_reference() });
                debug_assert!(is_alloced(unsafe { free_list.to_object_reference() }));
                trace!("alloc to {}", free_list);
                return free_list;
            }
            trace!("block {} exhausted, try next", block.start());
            available_blocks.pop::<VM>();
            self.consumed_blocks.get_mut(bin as usize).unwrap().push::<VM>(block);

            block = available_blocks.first;

        }
        unsafe { Address::zero() }
    }


    // pub fn block_thread_free_collect(&self, block: Address) {
    //     let free_list = block.load_free_list();

    //     let mut success = false;
    //     let mut thread_free = unsafe { Address::zero() };
    //     while !success {
    //         thread_free = FreeListAllocator::<VM>::load_thread_free_list(block);
    //         if thread_free.is_zero() {
    //             // no frees from other threads to worry about
    //             return
    //         }
    //         success = self.cas_thread_free_list(block, thread_free, unsafe { Address::zero() });
    //     }
    //     assert!(false);

    //     // no more CAS needed
    //     // futher frees to the thread free list will be done from a new empty list
    //     if !free_list.is_zero() {
    //         let mut tail = thread_free;
    //         unsafe {
    //             let mut next = tail.load::<Address>();
    //             while !next.is_zero() {
    //                 tail = next;
    //                 next = tail.load::<Address>();
    //             }
    //             tail.store(free_list);
    //         }
    //     }
    //     block.store_free_list(thread_free);
    // }

    pub fn block_free_collect(&self, block: Block) {
        let free_list = block.load_free_list::<VM>();

        // first, other threads
        // self.block_thread_free_collect(block);

        // same thread
        let local_free = block.load_local_free_list::<VM>();
        block.store_local_free_list::<VM>(unsafe{Address::zero()});
        debug_assert!(block.load_local_free_list::<VM>().is_zero());

        if !local_free.is_zero() {
            if !free_list.is_zero() {
                let mut tail = local_free;
                unsafe {
                    let mut next = tail.load::<Address>();
                    while !next.is_zero() {
                        tail = next;
                        next = tail.load::<Address>();
                    }
                    tail.store(free_list);
                }
            }
            block.store_free_list::<VM>(local_free);
        }

        debug_assert!(block.load_local_free_list::<VM>().is_zero());
    }



    pub fn acquire_block_for_size(&mut self, size: usize) -> Block {

        let bin = mi_bin(size) as usize;
        debug_assert!(self.available_blocks[bin].is_empty()); // only use this function if there are no blocks available

        // attempt to sweep
        #[cfg(not(feature = "eager_sweeping"))]
        loop {
            let block = self.unswept_blocks.get_mut(bin).unwrap().pop::<VM>();
            if block.is_zero() {
                // no more blocks to sweep
                break
            }
            self.sweep_block(block);
            if block.has_free_cells::<VM>() {
                // recyclable block
                self.available_blocks.get_mut(bin).unwrap().push::<VM>(block);
                eprintln!("push {} to available list, list = {:?}, list.lock = {}", block.start(), &self.available_blocks[bin] as *const _, &self.available_blocks[bin].lock.load(Ordering::SeqCst));
                return block;
            } else {
                // nothing was freed from this block
                self.consumed_blocks.get_mut(bin).unwrap().push::<VM>(block);
                eprintln!("push {} to consumed list, list = {:?}, list.lock = {}", block.start(), &self.consumed_blocks[bin] as *const _, &self.consumed_blocks[bin].lock.load(Ordering::SeqCst));
            }
        }
        self.acquire_fresh_block(size, false)
    }

    pub fn acquire_fresh_block(&mut self, size: usize, stress_test: bool) -> Block {
        // fresh block

        let block = Block::from(self.space.acquire(self.tls, Block::BYTES >> LOG_BYTES_IN_PAGE));
        if block.is_zero() {
            // GC
            return block;
        }
        self.space.record_new_block(block);

        // construct free list
        let block_end = block.start() + Block::BYTES;
        let mut old_cell = unsafe { Address::zero() };
        let mut new_cell = block.start();
        let block_list = if stress_test {
            self.available_blocks_stress.get_mut(mi_bin(size) as usize).unwrap()
        } else {
            self.available_blocks.get_mut(mi_bin(size) as usize).unwrap()
        };
        trace!(
            "Asked for size {}, construct free list with cells of size {}",
            size,
            block_list.size
        );
        assert!(size <= block_list.size);

        let final_cell = loop {
            unsafe {
                new_cell.store::<Address>(old_cell);
            }
            old_cell = new_cell;
            new_cell = new_cell + block_list.size;
            if new_cell + block_list.size > block_end {
                break old_cell;
            };
        };
        
        block.store_free_list::<VM>(final_cell);
        block.store_local_free_list::<VM>(unsafe { Address::zero() });
        block.store_thread_free_list::<VM>(unsafe { Address::zero() });
        block.store_block_cell_size::<VM>(block_list.size);
        block_list.push::<VM>(block);
        
        self.store_block_tls(block);
        trace!("Acquired fresh block starting at {}", block.start());
        block
    }

    pub fn sweep_block(&self, block: Block) {
        // eprintln!("s {}", block.start());
        let cell_size = block.load_block_cell_size::<VM>();
        // assert!(cell_size != 0, "b = {}, tls = {:?}, list = {:?}", block.start(), block.load_tls::<VM>(), block.load_block_list::<VM>() as *mut _);
        assert!(cell_size != 0);
        let mut cell = block.start();
        while cell < block.start() + Block::BYTES {
            let alloced = is_alloced(unsafe { cell.to_object_reference() });
            if alloced {
                let marked = is_marked::<VM>(
                    unsafe { cell.to_object_reference() },
                    Some(Ordering::SeqCst),
                );
                if !marked {
                    self.free(cell);
                }
            }
            cell += cell_size;
        }
        self.block_free_collect(block);
    }

    pub fn free(&self, addr: Address) {

        let block = Block::from(Block::align(addr));
        let block_tls = block.load_tls::<VM>();

        if self.tls.0 == block_tls {
            // same thread that allocated
            let local_free = block.load_local_free_list::<VM>();
            unsafe {
                addr.store(local_free);
            }
            block.store_local_free_list::<VM>(addr);
        } else {
            // different thread to allocator
            unreachable!("tlss don't match freeing from block {}, my tls = {:?}, block tls = {:?}", block.start(), self.tls, block.load_tls::<VM>());
            // let mut success = false;
            // while !success {
            //     let thread_free = FreeListAllocator::<VM>::load_thread_free_list(block);
            //     unsafe {
            //         addr.store(thread_free);
            //     }
            //     success = FreeListAllocator::<VM>::cas_thread_free_list(&self, block, thread_free, addr);
            // }
        }

        // unset allocation bit
        unsafe { unset_alloc_bit_unsafe(addr.to_object_reference()) };
    }

    pub fn store_block_tls(&self, block: Block) {
        block.store_tls::<VM>(self.tls);
    }

    #[cfg(not(feature = "eager_sweeping"))]
    pub fn reset(&mut self) {
        // consumed and available are now unswept
        let mut bin = 0;
        while bin < MI_BIN_HUGE + 1 {
            let unswept = self.unswept_blocks.get_mut(bin).unwrap();
            let available = self.available_blocks.get_mut(bin).unwrap();
            unswept.lock();
            available.lock();
            unswept.append::<VM>(available);
            available.unlock();
            let consumed = self.consumed_blocks.get_mut(bin).unwrap();
            consumed.lock();
            unswept.append::<VM>(consumed);
            consumed.unlock();
            let available_stress = self.available_blocks_stress.get_mut(bin).unwrap();
            available_stress.lock();
            unswept.append::<VM>(available_stress);
            available_stress.unlock();
            unswept.unlock();
            bin += 1;
        }
    }

    #[cfg(feature = "eager_sweeping")]
    pub fn reset(&mut self) {

        debug!("reset");
        // sweep all blocks and push consumed onto available list
        let mut bin = 0;
        while bin < MI_BIN_HUGE + 1 {
            let available = self.available_blocks.get_mut(bin).unwrap();

            let mut block = available.first;
            while !block.is_zero() {
                let next = block.load_next_block::<VM>();
                if !block.sweep(self.space) {
                    self.sweep_block(block);
                }
                block = next;
            }

            let consumed = self.consumed_blocks.get_mut(bin).unwrap();
            let mut block = consumed.first;
            while !block.is_zero() {
                self.sweep_block(block);
                block = block.load_next_block::<VM>();
            }

            self.available_blocks.get_mut(bin).unwrap().append::<VM>(self.consumed_blocks.get_mut(bin).unwrap());
            bin += 1;
        }
    }

    pub fn rebind(&mut self, space: &'static MarkSweepSpace<VM>) {
        trace!("rebind");
        self.reset();
        self.space = space;
    }


}

fn mi_wsize_from_size(size: usize) -> usize {
    // Align a byte size to a size in machine words
    // i.e. byte size == `wsize*sizeof(void*)`
    // adapted from _mi_wsize_from_size in mimalloc
    (size + size_of::<u32>() - 1) / size_of::<u32>()
}

pub fn mi_bin(size: usize) -> u8 {
    // adapted from _mi_bin in mimalloc
    let mut wsize: usize = mi_wsize_from_size(size);
    let bin: u8;
    if wsize <= 1 {
        bin = 1;
    } else if wsize <= 8 {
        bin = wsize as u8;
        // bin = ((wsize + 1) & !1) as u8; // round to double word sizes
    } else if wsize > MI_LARGE_OBJ_WSIZE_MAX {
        // bin = MI_BIN_HUGE;
        panic!(); // this should not be reached, because I'm sending objects bigger than this to the immortal space
    } else {
        wsize -= 1;
        let b = (MI_INTPTR_BITS - 1 - (u64::leading_zeros(wsize as u64)) as usize) as u8; // note: wsize != 0
        bin = ((b << 2) + ((wsize >> (b - 2)) & 0x03) as u8) - 3;
    }
    bin
}