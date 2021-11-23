use std::{mem::size_of, ops::BitAnd};

use atomic::Ordering;
use crate::policy::marksweepspace::block::Block;
use crate::policy::marksweepspace::metadata::is_marked;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::Space;
use crate::util::alloc_bit::{is_alloced, set_alloc_bit, unset_alloc_bit_unsafe};
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::metadata::load_metadata;
use crate::util::metadata::store_metadata;
use crate::util::metadata::MetadataSpec;
use crate::util::alloc::Allocator;
use crate::util::Address;
use crate::util::OpaquePointer;
use crate::util::VMThread;
use crate::vm::VMBinding;
use crate::Plan;

pub(crate) const BYTES_IN_BLOCK: usize = 1 << LOG_BYTES_IN_BLOCK;
pub(crate) const LOG_BYTES_IN_BLOCK: usize = 16;
const MI_BIN_HUGE: usize = 73;
const MI_INTPTR_SHIFT: usize = 3;
const MI_INTPTR_SIZE: usize = 1 << MI_INTPTR_SHIFT;
pub const MI_LARGE_OBJ_SIZE_MAX: usize = 1 << 21;
const MI_LARGE_OBJ_WSIZE_MAX: usize = MI_LARGE_OBJ_SIZE_MAX / MI_INTPTR_SIZE;
const MI_INTPTR_BITS: usize = MI_INTPTR_SIZE * 8;
const MI_BIN_FULL: usize = MI_BIN_HUGE + 1;

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

pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    available_blocks: Vec<BlockList>, // = pages in mimalloc
    #[cfg(feature = "lazy_sweeping")]
    unswept_blocks: Vec<BlockList>,
    consumed_blocks: Vec<BlockList>,
}

#[derive(Clone, Copy, Debug)]
pub struct BlockList {
    pub first: Address,
    pub last: Address,
    size: usize,
}

impl BlockList {
    const fn new(size: usize) -> BlockList {
        BlockList {
            first: unsafe { Address::zero() },
            last: unsafe { Address::zero() },
            size,
        }
    }

    fn is_empty(&self) -> bool {
        self.first.is_zero()
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
        // see mi_heap_malloc_small
        assert!(
            size <= BYTES_IN_BLOCK,
            "Alloc request for {} bytes is too big.",
            size
        );
        debug_assert!(align <= VM::MAX_ALIGNMENT);
        debug_assert!(align >= VM::MIN_ALIGNMENT);
        debug_assert!(offset == 0);

        let addr = self.alloc_from_available(size);
        if addr.is_zero() {
            debug_assert!(self.available_blocks[FreeListAllocator::<VM>::mi_bin(size) as usize].is_empty());
            return self.alloc_slow(size, align, offset)
        }
        addr
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // try to find an existing block with free cells
        let bin = FreeListAllocator::<VM>::mi_bin(size);
        if !self.available_blocks[bin as usize].is_empty() {
            // we've just had GC, which has made some blocks available
            return self.alloc_from_available(size);
        }

        let block = self.acquire_block_for_size(size);
        if block.is_zero() {
            // gc
            return block;
        }

        debug_assert!(!block.is_zero());

        // _mi_page_malloc
        let free_list = FreeListAllocator::<VM>::load_free_list(block);
        debug_assert!(!free_list.is_zero());

        // update free list
        let next_cell = unsafe { free_list.load::<Address>() };
        FreeListAllocator::<VM>::store_free_list(block, next_cell);
        debug_assert!(FreeListAllocator::<VM>::load_free_list(block) == next_cell);

        // set allocation bit
        set_alloc_bit(unsafe { free_list.to_object_reference() });
        debug_assert!(is_alloced(unsafe { free_list.to_object_reference() }));
        // eprintln!("a {}", free_list);

        free_list
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
            available_blocks: BLOCK_LISTS_EMPTY.to_vec(),
            #[cfg(feature = "lazy_sweeping")]
            unswept_blocks: BLOCK_LISTS_EMPTY.to_vec(),
            consumed_blocks: BLOCK_LISTS_EMPTY.to_vec(),
        }
    }

    fn alloc_from_available(&mut self, size: usize) -> Address {
        let bin = FreeListAllocator::<VM>::mi_bin(size);
        debug_assert!(bin <= MI_BIN_HUGE as u8);

        let available_blocks = &mut self.available_blocks[bin as usize];
        debug_assert!(available_blocks.size >= size);

        let mut block = available_blocks.first;

        while !block.is_zero() {
            let free_list = FreeListAllocator::<VM>::load_free_list(block);
            if !free_list.is_zero() {
                // update free list
                let next_cell = unsafe { free_list.load::<Address>() };
                FreeListAllocator::<VM>::store_free_list(block, next_cell);
                debug_assert!(FreeListAllocator::<VM>::load_free_list(block) == next_cell);
                // set allocation bit
                set_alloc_bit(unsafe { free_list.to_object_reference() });
                debug_assert!(is_alloced(unsafe { free_list.to_object_reference() }));

                return free_list;
            }
            available_blocks.first = FreeListAllocator::<VM>::load_next_block(block);
            FreeListAllocator::<VM>::store_block_list(available_blocks.first, available_blocks);

            FreeListAllocator::<VM>::push_onto_block_list(&mut self.consumed_blocks[bin as usize], block);
            block = available_blocks.first;

        }
        unsafe { Address::zero() }
    }

    #[inline]
    pub fn load_free_list(block: Address) -> Address {
        unsafe {
            Address::from_usize(load_metadata::<VM>(
                &MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
                block.to_object_reference(),
                None,
                None,
            ))
        }
    }

    #[inline]
    pub fn store_free_list(block: Address, free_list: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
            unsafe { block.to_object_reference() },
            free_list.as_usize(),
            None,
            None,
        );
    }

    #[inline]
    pub fn load_local_free_list(block: Address) -> Address {
        unsafe {
            Address::from_usize(load_metadata::<VM>(
                &MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
                block.to_object_reference(),
                None,
                None,
            ))
        }
    }

    #[inline]
    pub fn store_local_free_list(block: Address, local_free: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
            unsafe { block.to_object_reference() },
            local_free.as_usize(),
            None,
            None,
        );
    }

    #[inline]
    pub fn load_thread_free_list(block: Address) -> Address {
        unsafe {
            Address::from_usize(load_metadata::<VM>(
                &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
                block.to_object_reference(),
                None,
                Some(Ordering::SeqCst),
            ))
        }
    }

    #[inline]
    pub fn store_thread_free_list(block: Address, thread_free: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
            unsafe { block.to_object_reference() },
            thread_free.as_usize(),
            None,
            None,
        );
    }

    // #[inline]
    // pub fn cas_thread_free_list(
    //     &self,
    //     block: Address,
    //     old_thread_free: Address,
    //     new_thread_free: Address,
    // ) -> bool {
    //     compare_exchange_metadata::<VM>(
    //         &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
    //         unsafe { block.to_object_reference() },
    //         old_thread_free.as_usize(),
    //         new_thread_free.as_usize(),
    //         None,
    //         Ordering::SeqCst,
    //         Ordering::SeqCst,
    //     )
    // }

    pub fn load_prev_block(block: Address) -> Address {
        assert!(!block.is_zero());
        let prev = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
            unsafe { block.to_object_reference() },
            None,
            None,
        );
        unsafe { Address::from_usize(prev) }
    }

    pub fn load_next_block(block: Address) -> Address {
        assert!(!block.is_zero());
        let next = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
            unsafe { block.to_object_reference() },
            None,
            None,
        );
        unsafe { Address::from_usize(next) }
    }

    pub fn store_next_block(block: Address, next: Address) {
        assert!(!block.is_zero());
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
            unsafe { block.to_object_reference() },
            next.as_usize(),
            None,
            None,
        );
    }

    pub fn store_prev_block(block: Address, prev: Address) {
        assert!(!block.is_zero());
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
            unsafe { block.to_object_reference() },
            prev.as_usize(),
            None,
            None,
        );
    }

    pub fn store_block_list(block: Address, block_list: *mut BlockList) {
        todo!()
    }

    pub fn load_block_list(block: Address) -> BlockList {
        todo!()
    }

    pub fn load_block_cell_size(block: Address) -> usize {
        load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::SIZE_TABLE),
            unsafe { block.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        )
    }
    
    pub fn store_block_cell_size(block: Address, size: usize) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::SIZE_TABLE),
            unsafe { block.to_object_reference() },
            size,
            None,
            None,
        );
    }

    fn pop_from_block_list(block_list: &mut BlockList) -> Address {
        let rtn = block_list.first;
        if rtn.is_zero() {
            return rtn;
        }
        let next = FreeListAllocator::<VM>::load_next_block(rtn);
        if next.is_zero() {
            block_list.first = unsafe { Address::zero() };
            block_list.last = unsafe { Address::zero() };
        } else {
            block_list.first = next;
            FreeListAllocator::<VM>::store_prev_block(next, unsafe {Address::zero()});
            FreeListAllocator::<VM>::store_block_list(block_list.first, block_list);
        }
        rtn
    }

    fn push_onto_block_list(block_list: &mut BlockList, block: Address) {
        if block_list.first.is_zero() {
            block_list.first = block;
            block_list.last = block;
        } else {
            FreeListAllocator::<VM>::store_next_block(block, block_list.first);
            block_list.first = block;
        }
        FreeListAllocator::<VM>::store_block_list(block, block_list);
    }

    // pub fn block_thread_free_collect(&self, block: Address) {
    //     let free_list = FreeListAllocator::<VM>::load_free_list(block);

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
    //     FreeListAllocator::<VM>::store_free_list(block, thread_free);
    // }

    pub fn block_free_collect(&self, block: Address) {
        let free_list = FreeListAllocator::<VM>::load_free_list(block);

        // first, other threads
        // self.block_thread_free_collect(block);

        // same thread
        let local_free = FreeListAllocator::<VM>::load_local_free_list(block);
        FreeListAllocator::<VM>::store_local_free_list(block, unsafe { Address::zero() });
        debug_assert!(FreeListAllocator::<VM>::load_local_free_list(block).is_zero());

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
            FreeListAllocator::<VM>::store_free_list(block, local_free);
        }

        debug_assert!(FreeListAllocator::<VM>::load_local_free_list(block).is_zero());
    }

    pub fn block_has_free_cells(block: Address) -> bool {
        debug_assert!(!block.is_zero());
        !FreeListAllocator::<VM>::load_free_list(block).is_zero()

    }

    pub fn acquire_block_for_size(&mut self, size: usize) -> Address {

        let bin = FreeListAllocator::<VM>::mi_bin(size) as usize;
        debug_assert!(self.available_blocks[bin].is_empty()); // only use this function if there are no blocks available

        // attempt to sweep
        #[cfg(feature = "lazy_sweeping")]
        loop {
            let block = FreeListAllocator::<VM>::pop_from_block_list(self.unswept_blocks.get_mut(bin).unwrap());
            if block.is_zero() {
                // no more blocks to sweep
                break
            }
            self.sweep_block(block);
            if FreeListAllocator::<VM>::block_has_free_cells(block) {
                // recyclable block
                FreeListAllocator::<VM>::push_onto_block_list(
                    self.available_blocks.get_mut(bin).unwrap(),
                    block,
                );
                return block;
            } else {
                // nothing was freed from this block
                FreeListAllocator::<VM>::push_onto_block_list(
                    self.consumed_blocks.get_mut(bin).unwrap(),
                    block,
                );
            }
        }

        // fresh block

        let block = self.space.acquire(self.tls, BYTES_IN_BLOCK >> LOG_BYTES_IN_PAGE);

        if block.is_zero() {
            // GC
            return block;
        }
        self.space.record_new_block(block);
        eprintln!("b > 0x{:0x} {}", block, self.available_blocks[bin as usize].size);

        // construct free list
        let block_end = block + BYTES_IN_BLOCK;
        let mut old_cell = unsafe { Address::zero() };
        let mut new_cell = block;
        let block_list = self.available_blocks.get_mut(FreeListAllocator::<VM>::mi_bin(size) as usize).unwrap();
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
        
        FreeListAllocator::<VM>::store_free_list(block, final_cell);
        FreeListAllocator::<VM>::store_local_free_list(block, unsafe { Address::zero() });
        FreeListAllocator::<VM>::store_thread_free_list(block, unsafe { Address::zero() });
        FreeListAllocator::<VM>::store_block_cell_size(block, block_list.size);
        FreeListAllocator::<VM>::push_onto_block_list(block_list, block);
        self.store_block_tls(block);
        trace!("Constructed free list for block starting at {}", block);
        block
    }

    pub fn get_block(addr: Address) -> Address {
        unsafe { Address::from_usize(addr.bitand(!0xFFFF as usize)) }
    }

    pub fn sweep_block(&self, block: Address) {
        let cell_size = FreeListAllocator::<VM>::load_block_cell_size(block);
        assert!(cell_size != 0, "b = {}", block);
        let mut cell = block;
        while cell < block + BYTES_IN_BLOCK {
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
        // eprintln!("f {}", addr);

        let block = FreeListAllocator::<VM>::get_block(addr);
        let block_tls = self.space.load_block_tls(block);

        if self.tls.0 == block_tls {
            // same thread that allocated
            let local_free = FreeListAllocator::<VM>::load_local_free_list(block);
            unsafe {
                addr.store(local_free);
            }
            FreeListAllocator::<VM>::store_local_free_list(block, addr);
        } else {
            // different thread to allocator
            unreachable!();
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

    pub fn store_block_tls(&self, block: Address) {
        let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(self.tls.0) };
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::TLS_TABLE),
            unsafe { block.to_object_reference() },
            tls,
            None,
            None,
        );
    }

    #[cfg(feature = "lazy_sweeping")]
    pub fn reset(&mut self) {
        // use crate::policy::marksweepspace::block::BlockState;

        trace!("reset");
        // consumed and available are now unswept
        let mut bin = 0;
        while bin < MI_BIN_HUGE + 1 {
            let unswept = self.unswept_blocks.get_mut(bin).unwrap();

            let available = self.available_blocks[bin];
            debug_assert!(available.size == unswept.size);
            if !available.is_empty() {
                if unswept.is_empty() {
                    unswept.first = available.first;
                    FreeListAllocator::<VM>::store_block_list(unswept.first, unswept);
                } else {
                    FreeListAllocator::<VM>::store_next_block(
                        unswept.last,
                        available.first,
                    );
                }
                unswept.last = available.last;
                debug_assert!(!unswept.first.is_zero());
                debug_assert!(FreeListAllocator::<VM>::load_next_block(unswept.last).is_zero());
            }
            let consumed = self.consumed_blocks[bin];
            if !consumed.is_empty() {
                if unswept.is_empty() {
                    unswept.first = consumed.first;
                    FreeListAllocator::<VM>::store_block_list(unswept.first, unswept);
                } else {
                    FreeListAllocator::<VM>::store_next_block(
                        unswept.last,
                        consumed.first,
                    );
                }
                unswept.last = consumed.last;
            }

            let mut prev = unsafe { Address::zero() };
            let mut unswept_b = unswept.first;
            while !unswept_b.is_zero() {
                FreeListAllocator::<VM>::store_block_list(unswept_b, unswept);
                unswept_b = FreeListAllocator::<VM>::load_next_block(unswept_b);
            }

            bin += 1;
        }


        self.available_blocks = BLOCK_LISTS_EMPTY.to_vec();
        self.consumed_blocks = BLOCK_LISTS_EMPTY.to_vec();
    }

    #[cfg(not(feature = "lazy_sweeping"))]
    pub fn reset(&mut self) {
        trace!("reset");
        // sweep all blocks and push consumed onto available list
        let mut bin = 0;
        while bin < MI_BIN_HUGE + 1 {
            let available = self.available_blocks[bin];
            let consumed = self.consumed_blocks[bin];
            if !available.first.is_zero() {
                let mut block = available.first;
                self.sweep_block(block);
                let mut next = FreeListAllocator::<VM>::load_next_block(block);
                while !next.is_zero() {
                    block = next;
                    self.sweep_block(block);
                    next = FreeListAllocator::<VM>::load_next_block(block);
                }
                FreeListAllocator::<VM>::store_next_block(
                    block,
                    consumed.first,
                );
            } else {
                self.available_blocks[bin].first = consumed.first;
                FreeListAllocator::<VM>::store_block_list(self.available_blocks[bin].first, self.available_blocks[bin]);
            }
            if !consumed.first.is_zero() {
                let mut block = consumed.first;
                while !block.is_zero() {
                    self.sweep_block(block);
                    block = FreeListAllocator::<VM>::load_next_block(block);
                }
            }
            bin += 1;
        }
        self.consumed_blocks = BLOCK_LISTS_EMPTY.to_vec();
    }

    pub fn rebind(&mut self, space: &'static MarkSweepSpace<VM>) {
        trace!("rebind");
        self.reset();
        self.space = space;
    }

    fn mi_wsize_from_size(size: usize) -> usize {
        // Align a byte size to a size in machine words
        // i.e. byte size == `wsize*sizeof(void*)`
        // adapted from _mi_wsize_from_size in mimalloc
        (size + size_of::<u32>() - 1) / size_of::<u32>()
    }

    pub fn mi_bin(size: usize) -> u8 {
        // adapted from _mi_bin in mimalloc
        let mut wsize: usize = FreeListAllocator::<VM>::mi_wsize_from_size(size);
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
}
