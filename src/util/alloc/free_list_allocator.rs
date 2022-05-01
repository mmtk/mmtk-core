use std::sync::atomic::AtomicBool;
use std::mem::{size_of, size_of_val};

use atomic::Ordering;
use atomic_traits::Atomic;
use crate::policy::marksweepspace::block::Block;
use crate::policy::marksweepspace::metadata::is_marked;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::Space;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::alloc::Allocator;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;
use crate::vm::ActivePlan;
use crate::Plan;
use libc::c_void;

const MI_INTPTR_SHIFT: usize = 3;
const MI_INTPTR_SIZE: usize = 1 << MI_INTPTR_SHIFT;
pub const MI_LARGE_OBJ_SIZE_MAX: usize = 1 << 21;
pub const MI_LARGE_OBJ_WSIZE_MAX: usize = MI_LARGE_OBJ_SIZE_MAX / MI_INTPTR_SIZE;
const MI_INTPTR_BITS: usize = MI_INTPTR_SIZE * 8;
pub const MI_BIN_FULL: usize = MAX_BIN + 1;
pub const BYTES_IN_BLOCK_WSIZE: usize = Block::BYTES / MI_INTPTR_SIZE;
pub const MAX_BIN: usize = 48;
const ZERO_BLOCK: Block = Block::from(unsafe { Address::zero() });
pub type BlockLists = [BlockList; MAX_BIN + 1];

// mimalloc init.c:46
pub(crate) const BLOCK_LISTS_EMPTY: BlockLists = [
    BlockList::new(1 * MI_INTPTR_SIZE),
    BlockList::new(1 * MI_INTPTR_SIZE),
    BlockList::new(2 * MI_INTPTR_SIZE),
    BlockList::new(3 * MI_INTPTR_SIZE),
    BlockList::new(4 * MI_INTPTR_SIZE),
    BlockList::new(5 * MI_INTPTR_SIZE),
    BlockList::new(6 * MI_INTPTR_SIZE),
    BlockList::new(7 * MI_INTPTR_SIZE),
    BlockList::new(8 * MI_INTPTR_SIZE), /* 8 */
    BlockList::new(10 * MI_INTPTR_SIZE),
    BlockList::new(12 * MI_INTPTR_SIZE),
    BlockList::new(14 * MI_INTPTR_SIZE),
    BlockList::new(16 * MI_INTPTR_SIZE),
    BlockList::new(20 * MI_INTPTR_SIZE),
    BlockList::new(24 * MI_INTPTR_SIZE),
    BlockList::new(28 * MI_INTPTR_SIZE),
    BlockList::new(32 * MI_INTPTR_SIZE), /* 16 */
    BlockList::new(40 * MI_INTPTR_SIZE),
    BlockList::new(48 * MI_INTPTR_SIZE),
    BlockList::new(56 * MI_INTPTR_SIZE),
    BlockList::new(64 * MI_INTPTR_SIZE),
    BlockList::new(80 * MI_INTPTR_SIZE),
    BlockList::new(96 * MI_INTPTR_SIZE),
    BlockList::new(112 * MI_INTPTR_SIZE),
    BlockList::new(128 * MI_INTPTR_SIZE), /* 24 */
    BlockList::new(160 * MI_INTPTR_SIZE),
    BlockList::new(192 * MI_INTPTR_SIZE),
    BlockList::new(224 * MI_INTPTR_SIZE),
    BlockList::new(256 * MI_INTPTR_SIZE),
    BlockList::new(320 * MI_INTPTR_SIZE),
    BlockList::new(384 * MI_INTPTR_SIZE),
    BlockList::new(448 * MI_INTPTR_SIZE),
    BlockList::new(512 * MI_INTPTR_SIZE), /* 32 */
    BlockList::new(640 * MI_INTPTR_SIZE),
    BlockList::new(768 * MI_INTPTR_SIZE),
    BlockList::new(896 * MI_INTPTR_SIZE),
    BlockList::new(1024 * MI_INTPTR_SIZE),
    BlockList::new(1280 * MI_INTPTR_SIZE),
    BlockList::new(1536 * MI_INTPTR_SIZE),
    BlockList::new(1792 * MI_INTPTR_SIZE),
    BlockList::new(2048 * MI_INTPTR_SIZE), /* 40 */
    BlockList::new(2560 * MI_INTPTR_SIZE),
    BlockList::new(3072 * MI_INTPTR_SIZE),
    BlockList::new(3584 * MI_INTPTR_SIZE),
    BlockList::new(4096 * MI_INTPTR_SIZE),
    BlockList::new(5120 * MI_INTPTR_SIZE),
    BlockList::new(6144 * MI_INTPTR_SIZE),
    BlockList::new(7168 * MI_INTPTR_SIZE),
    BlockList::new(8192 * MI_INTPTR_SIZE), /* 48 */
];


#[repr(C)]
pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    pub available_blocks: BlockLists, // = pages in mimalloc
    pub available_blocks_stress: BlockLists,
    pub unswept_blocks: BlockLists,
    pub consumed_blocks: BlockLists,
}

#[derive(Debug)]
#[repr(C)]
pub struct BlockList {
    pub first: Block,
    pub last: Block,
    pub size: usize,
    pub lock: AtomicBool,
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

    pub fn is_empty(&self) -> bool {
        self.first.is_zero()
    }

    pub fn remove<VM: VMBinding>(&mut self, block: Block) {
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

    pub fn pop<VM: VMBinding>(&mut self) -> Block {
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
    
    pub fn append<VM: VMBinding>(&mut self, list: &mut BlockList) {
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
                block.store_block_list::<VM>(self);
                block = block.load_next_block::<VM>();
            }
            list.reset();
        }
    }

    fn reset(&mut self) {
        self.first = ZERO_BLOCK;
        self.last = ZERO_BLOCK;
    }

    pub fn lock(&mut self) {
        assert!(self.size <= MI_LARGE_OBJ_SIZE_MAX, "{:?} {}", self as *mut _, self.size);
        let mut success = false;
        while !success {
            success = self.lock.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok();
        }
    }

    pub fn unlock(&mut self) {
        self.lock.store(false, Ordering::SeqCst);
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
        assert!(align <= VM::MAX_ALIGNMENT);
        assert!(align >= VM::MIN_ALIGNMENT);
        assert!(offset == 0);

        let block = self.find_free_block(size);
        let addr = self.block_alloc(block, size, align, offset);
        if addr.is_zero() {
            self.alloc_slow(size, align, offset)
        } else {
            addr
        }


    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {

        let block = self.find_free_block(size);
        if block.is_zero() {
            return unsafe {Address::zero()};
        }
    
        self.block_alloc(block, size, align, offset)

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
        assert!(self.available_blocks[bin].is_empty());
        if need_poll {
            VM::VMActivePlan::global().poll(false, self.space);
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
                    assert!(block.load_free_list::<VM>() == next_cell);
    
                    return free_list;
                }
                available.pop::<VM>();
                self.consumed_blocks.get_mut(bin as usize).unwrap().push::<VM>(block);
    
                block = available.first;
    
            }
        }
        let block = self.acquire_block_for_size(size, true);
        if block.is_zero() {
            return unsafe {Address::zero()};
        }

        let free_list = block.load_free_list::<VM>();
        block.store_free_list::<VM>(unsafe { free_list.load::<Address>() });

        // // set allocation bit
        // set_alloc_bit(unsafe { free_list.to_object_reference() });

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
        eprintln!("new free list allocator");
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

    pub fn block_alloc(&mut self, block: Block, size: usize, align: usize, offset: isize) -> Address {
        if block.is_zero() {
            return unsafe { Address::zero() }
        }
        let cell = block.load_free_list::<VM>();
        if cell.is_zero() {
            return self.alloc_slow(size, align, offset);
        }
        block.store_free_list::<VM>(unsafe { cell.load::<Address>() });
        cell
    }

    fn find_free_block(&mut self, size: usize) -> Block {
        let bin = mi_bin(size);
        assert!(bin <= MAX_BIN);

        let available_blocks = &mut self.available_blocks[bin as usize];
        assert!(available_blocks.size >= size);

        let mut block = available_blocks.first;

        while !block.is_zero() {
            if block.has_free_cells::<VM>() {
                return block;
            }
            available_blocks.pop::<VM>();
            self.consumed_blocks.get_mut(bin as usize).unwrap().push::<VM>(block);

            block = available_blocks.first;

        }
        
        self.acquire_block_for_size(size, false)

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

    // pub fn block_free_collect(&self, block: Block) {
    //     let free_list = block.load_free_list::<VM>();

    //     // first, other threads
    //     // self.block_thread_free_collect(block);

    //     // same thread
    //     let local_free = block.load_local_free_list::<VM>();
    //     block.store_local_free_list::<VM>(unsafe{Address::zero()});
    //     assert!(block.load_local_free_list::<VM>().is_zero());

    //     if !local_free.is_zero() {
    //         if !free_list.is_zero() {
    //             let mut tail = local_free;
    //             unsafe {
    //                 let mut next = tail.load::<Address>();
    //                 while !next.is_zero() {
    //                     tail = next;
    //                     next = tail.load::<Address>();
    //                 }
    //                 tail.store(free_list);
    //             }
    //         }
    //         block.store_free_list::<VM>(local_free);
    //     }

    //     assert!(block.load_local_free_list::<VM>().is_zero());
    // }



    pub fn acquire_block_for_size(&mut self, size: usize, stress_test: bool) -> Block {

        let bin = mi_bin(size) as usize;
        assert!(self.available_blocks[bin].is_empty()); // only use this function if there are no blocks available

        // attempt to sweep
        #[cfg(not(feature = "eager_sweeping"))]
        loop {
            let block = self.unswept_blocks.get_mut(bin).unwrap().pop::<VM>();
            if block.is_zero() {

            if block.is_zero() {

        eprintln!("block for size c blocks: {:?}", self.consumed_blocks[10]);
        // attempt from unswept blocks
        let bin = FreeListAllocator::<VM>::mi_bin(size) as usize;
        // eprintln!("available blocks: {:?}", self.available_blocks[bin]);
        // eprintln!("unswept blocks: {:?}", self.unswept_blocks[bin]);
        // eprintln!("consumed blocks: {:?}", self.consumed_blocks[bin]);

        loop {
            unreachable!();
            let block = FreeListAllocator::<VM>::pop_from_block_list(self.unswept_blocks.get_mut(bin).unwrap());
            if block.is_zero() {
                // reached end of unswept list
                break
            }
            self.sweep_block(block);
            if block.has_free_cells::<VM>() {
                // recyclable block
                if stress_test {
                    self.available_blocks_stress.get_mut(bin).unwrap().push::<VM>(block);
                } else {
                    self.available_blocks.get_mut(bin).unwrap().push::<VM>(block);
                }
                return block;
            } else {
                // nothing was freed from this block
                self.consumed_blocks.get_mut(bin).unwrap().push::<VM>(block);
            }
        }
        self.acquire_block(size, stress_test)
    }

    pub fn acquire_block(&mut self, size: usize, stress_test: bool) -> Block {
        // fresh block
        let bin = mi_bin(size);
        loop {
            match self.space.acquire_block(self.tls, size) {
                crate::policy::marksweepspace::BlockAcquireResult::Fresh(block) => {
                    if block.is_zero() {
                        // GC
                        return block;
                    }
                    if stress_test {
                        self.available_blocks_stress[bin].push::<VM>(block);
                    } else {
                        self.available_blocks[bin].push::<VM>(block);
                    }
                    self.init_block(block, self.available_blocks[bin].size);

                    return block;
                },

                crate::policy::marksweepspace::BlockAcquireResult::AbandonedAvailable(block) => {
                    block.store_tls::<VM>(self.tls);
                    if block.has_free_cells::<VM>() {
                        if stress_test {
                            self.available_blocks_stress[bin].push::<VM>(block);
                        } else {
                            self.available_blocks[bin].push::<VM>(block);
                        }
                        return block
                    } else {
                        self.consumed_blocks[bin].push::<VM>(block);
                    }

                },

                crate::policy::marksweepspace::BlockAcquireResult::AbandondedUnswept(block) => {
                    block.store_tls::<VM>(self.tls);
                    self.sweep_block(block);
                    if block.has_free_cells::<VM>() {
                        if stress_test {
                            self.available_blocks_stress[bin].push::<VM>(block);
                        } else {
                            self.available_blocks[bin].push::<VM>(block);
                        }
                        return block
                    } else {
                        self.consumed_blocks[bin].push::<VM>(block);
                    }
                },
            }
        };
    }

    pub fn init_block(&self, block: Block, cell_size: usize) {

        self.space.record_new_block(block);

        // construct free list
        let block_end = block.start() + BYTES_IN_BLOCK;
        let mut old_cell = unsafe { Address::zero() };
        let mut new_cell = block.start();

        let final_cell = loop {
            unsafe {
                new_cell.store::<Address>(old_cell);
            }
            old_cell = new_cell;
            new_cell = new_cell + cell_size;
            if new_cell + cell_size > block_end {
                break old_cell;
            };
        };
        
        block.store_free_list::<VM>(final_cell);
        // block.store_local_free_list::<VM>(unsafe { Address::zero() });
        // block.store_thread_free_list::<VM>(unsafe { Address::zero() });
        block.store_block_cell_size::<VM>(cell_size);

        self.store_block_tls(block);
        
    }

    pub fn sweep_block(&self, block: Block) {

        let cell_size = block.load_block_cell_size::<VM>();
        let mut cell = block.start();
        let mut last = unsafe { Address::zero() };
        while cell + cell_size <= block.start() + Block::BYTES {
            if !is_marked::<VM>(
                unsafe { cell.to_object_reference() },
                Some(Ordering::SeqCst),
            ) {
                unsafe { cell.store::<Address>(last); }
                last = cell;
            }
            cell += cell_size;
        }
        block.store_free_list::<VM>(last);
    }

        // let cell_size = block.load_block_cell_size::<VM>();
        // assert!(cell_size != 0);
        // let mut cell = block.start();
        // while cell < block.start() + Block::BYTES {
        //     let alloced = is_alloced(unsafe { cell.to_object_reference() });
        //     if alloced {
        //         let marked = is_marked::<VM>(
        //             unsafe { cell.to_object_reference() },
        //             Some(Ordering::SeqCst),
        //         );
        //         if !marked {
        //             self.free(cell);
        //         }
        //     }
        //     cell += cell_size;
        // }
        // self.block_free_collect(block);
    // }

    // pub fn free(&self, addr: Address) {

    //     let block = Block::from(Block::align(addr));
    //     let block_tls = block.load_tls::<VM>();

    //     if self.tls.0 == block_tls {
    //         // same thread that allocated
    //         let local_free = block.load_local_free_list::<VM>();
    //         unsafe {
    //             addr.store(local_free);
    //         }
    //         block.store_local_free_list::<VM>(addr);
    //     } else {
    //         // different thread to allocator
    //         unreachable!("tlss don't match freeing from block {}, my tls = {:?}, block tls = {:?}", block.start(), self.tls, block.load_tls::<VM>());
    //         // let mut success = false;
    //         // while !success {
    //         //     let thread_free = FreeListAllocator::<VM>::load_thread_free_list(block);
    //         //     unsafe {
    //         //         addr.store(thread_free);
    //         //     }
    //         //     success = FreeListAllocator::<VM>::cas_thread_free_list(&self, block, thread_free, addr);
    //         // }
    //     }

    //     // unset allocation bit
    //     unsafe { unset_alloc_bit_unsafe(addr.to_object_reference()) };
    // }

    pub fn store_block_tls(&self, block: Block) {
        block.store_tls::<VM>(self.tls);
    }

    #[cfg(not(feature = "eager_sweeping"))]
    pub fn reset(&mut self) {
        trace!("reset");
        // consumed and available are now unswept
        let mut bin = 0;
        while bin < MAX_BIN + 1 {
            let unswept = self.unswept_blocks.get_mut(bin).unwrap();
            unswept.lock();
            let available = self.available_blocks.get_mut(bin).unwrap();
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
        use crate::policy::marksweepspace::block::BlockState;

        trace!("reset");
        // sweep all blocks and push consumed onto available list
        let mut bin = 0;
        while bin < MAX_BIN + 1 {
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

    pub fn abandon_blocks(&mut self) {
        let mut abandoned = self.space.abandoned_available.lock().unwrap();
        let mut abandoned_consumed = self.space.abandoned_consumed.lock().unwrap();
        let mut i = 0;
        while i < MI_BIN_FULL {
            let available = self.available_blocks.get_mut(i).unwrap();
            if !available.is_empty() {
                // let mut block = abandoned.first().unwrap();
                // while !block.is_zero() {
                //     print("abandon available block {}", block.start());
                // }
                abandoned[i].append::<VM>(available);
            }
            
            let available_stress = self.available_blocks_stress.get_mut(i).unwrap();
            if !available_stress.is_empty() {
                abandoned[i].append::<VM>(available_stress);
            }

            let consumed = self.consumed_blocks.get_mut(i).unwrap();
            if !consumed.is_empty() {
                abandoned_consumed[i].append::<VM>(consumed);
            }

            let unswept = self.unswept_blocks.get_mut(i).unwrap();
            if !unswept.is_empty() {
                abandoned[i].append::<VM>(unswept);
            }
            i = i + 1;
        }
    }
}

fn mi_wsize_from_size(size: usize) -> usize {
    // Align a byte size to a size in machine words
    // i.e. byte size == `wsize*sizeof(void*)`
    // adapted from _mi_wsize_from_size in mimalloc
    let r = (size + MI_INTPTR_SIZE - 1) / MI_INTPTR_SIZE;
    // eprintln!("mwfs {} -> {}", size, r);
    r
}

pub fn mi_bin(size: usize) -> usize {
    // adapted from _mi_bin in mimalloc
    let mut wsize: usize = mi_wsize_from_size(size);
    let bin: u8;
    if wsize <= 1 {
        bin = 1;
    } else if wsize <= 8 {
        bin = wsize as u8;
        // bin = ((wsize + 1) & !1) as u8; // round to double word sizes
    } else if wsize > MI_LARGE_OBJ_WSIZE_MAX {
        // bin = MAX_BIN;
        panic!(); // this should not be reached, because I'm sending objects bigger than this to the immortal space
    } else {
        wsize -= 1;
        let b = (MI_INTPTR_BITS - 1 - (u64::leading_zeros(wsize as u64)) as usize) as u8; // note: wsize != 0
        bin = ((b << 2) + ((wsize >> (b - 2)) & 0x03) as u8) - 3;
    }
    // eprintln!("mb {} -> {}", size, bin);
    bin as usize
}