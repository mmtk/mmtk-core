// This is a free list allocator written based on Microsoft's mimalloc allocator https://www.microsoft.com/en-us/research/publication/mimalloc-free-list-sharding-in-action/

use crate::policy::marksweepspace::native_ms::Block;
// use crate::policy::marksweepspace::metadata::is_marked;
use crate::policy::marksweepspace::native_ms::MarkSweepSpace;
use crate::util::alloc::allocator;
use crate::util::alloc::Allocator;
use crate::util::linear_scan::Region;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;
use crate::Plan;
use atomic::Ordering;
use std::sync::atomic::AtomicBool;

/// Log2 of pointer size
const MI_INTPTR_SHIFT: usize = crate::util::constants::LOG_BYTES_IN_ADDRESS as usize;
/// pointer size in bytes
const MI_INTPTR_SIZE: usize = 1 << MI_INTPTR_SHIFT;
/// pointer size in bits
const MI_INTPTR_BITS: usize = MI_INTPTR_SIZE * 8;
/// Number of bins in BlockLists. Reserve bin0 as an empty bin.
pub(crate) const MI_BIN_FULL: usize = MAX_BIN + 1;
/// The largest valid bin.
pub(crate) const MAX_BIN: usize = 48;

const ZERO_BLOCK: Block = Block::ZERO_BLOCK;

/// Largest object size allowed with our mimalloc implementation, in bytes
pub(crate) const MI_LARGE_OBJ_SIZE_MAX: usize = MAX_BIN_SIZE;
/// Largest object size in words
const MI_LARGE_OBJ_WSIZE_MAX: usize = MI_LARGE_OBJ_SIZE_MAX / MI_INTPTR_SIZE;
/// The object size for the last bin. We should not try allocate objects larger than this with the allocator.
const MAX_BIN_SIZE: usize = 8192 * MI_INTPTR_SIZE;

/// All the bins for the block lists
// Each block list takes roughly 8bytes * 4 * 49 = 1658 bytes. It is more reasonable to heap allocate them, and
// just put them behind a boxed pointer.
pub type BlockLists = Box<[BlockList; MAX_BIN + 1]>;

/// Create an empty set of block lists of different size classes (bins)
pub(crate) fn new_empty_block_lists() -> BlockLists {
    let ret = Box::new([
        BlockList::new(MI_INTPTR_SIZE),
        BlockList::new(MI_INTPTR_SIZE),
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
    ]);

    debug_assert_eq!(
        ret[MAX_BIN].size, MAX_BIN_SIZE,
        "MAX_BIN_SIZE = {}, actual max bin size  = {}, please update the constants",
        MAX_BIN_SIZE, ret[MAX_BIN].size
    );

    ret
}

// Free list allocator
#[repr(C)]
pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    /// blocks with free space
    pub available_blocks: BlockLists,
    /// blocks with free space for precise stress GC
    /// For precise stress GC, we need to be able to trigger slowpath allocation for
    /// each allocation. To achieve this, we put available blocks to this list. So
    /// normal fastpath allocation will fail, as they will see the normal
    /// as empty.
    pub available_blocks_stress: BlockLists,
    /// blocks that are marked, not swept
    pub unswept_blocks: BlockLists,
    /// full blocks
    pub consumed_blocks: BlockLists,
}

// List of blocks owned by the allocator
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

    // List has no blocks
    pub fn is_empty(&self) -> bool {
        self.first.is_zero()
    }

    // Remove a block from the list
    pub fn remove(&mut self, block: Block) {
        let prev = block.load_prev_block();
        let next = block.load_next_block();
        #[allow(clippy::collapsible_else_if)]
        if prev.is_zero() {
            if next.is_zero() {
                self.first = ZERO_BLOCK;
                self.last = ZERO_BLOCK;
            } else {
                next.store_prev_block(ZERO_BLOCK);
                self.first = next;
                next.store_block_list(self);
            }
        } else {
            if next.is_zero() {
                prev.store_next_block(next);
                prev.store_next_block(ZERO_BLOCK);
                self.last = prev;
                prev.store_block_list(self);
            } else {
                prev.store_next_block(next);
                next.store_prev_block(prev);
            }
        }
    }

    // Pop the first block in the list
    pub fn pop(&mut self) -> Block {
        let rtn = self.first;
        if rtn.is_zero() {
            return rtn;
        }
        let next = rtn.load_next_block();
        if next.is_zero() {
            self.first = ZERO_BLOCK;
            self.last = ZERO_BLOCK;
        } else {
            self.first = next;
            next.store_prev_block(ZERO_BLOCK);
            self.first.store_block_list(self);
        }
        rtn.store_next_block(ZERO_BLOCK);
        rtn.store_prev_block(ZERO_BLOCK);
        rtn
    }

    // Push block to the front of the list
    fn push(&mut self, block: Block) {
        if self.is_empty() {
            block.store_next_block(ZERO_BLOCK);
            block.store_prev_block(ZERO_BLOCK);
            self.first = block;
            self.last = block;
        } else {
            block.store_next_block(self.first);
            self.first.store_prev_block(block);
            block.store_prev_block(ZERO_BLOCK);
            self.first = block;
        }
        block.store_block_list(self);
    }

    // Append one block list to another
    // The second block list left empty
    pub fn append(&mut self, list: &mut BlockList) {
        if !list.is_empty() {
            debug_assert!(
                list.first.load_prev_block().is_zero(),
                "{} -> {}",
                list.first.load_prev_block().start(),
                list.first.start()
            );
            if self.is_empty() {
                self.first = list.first;
                self.last = list.last;
            } else {
                debug_assert!(
                    self.first.load_prev_block().is_zero(),
                    "{} -> {}",
                    self.first.load_prev_block().start(),
                    self.first.start()
                );
                self.last.store_next_block(list.first);
                list.first.store_prev_block(self.last);
                self.last = list.last;
            }
            let mut block = list.first;
            while !block.is_zero() {
                block.store_block_list(self);
                block = block.load_next_block();
            }
            list.reset();
        }
    }

    // Remove all blocks
    fn reset(&mut self) {
        self.first = ZERO_BLOCK;
        self.last = ZERO_BLOCK;
    }

    // Lock list
    pub fn lock(&mut self) {
        let mut success = false;
        while !success {
            success = self
                .lock
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
        }
    }

    // Unlock list
    pub fn unlock(&mut self) {
        self.lock.store(false, Ordering::SeqCst);
    }
}

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

    // Find a block with free space and allocate to it
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // trace!("alloc s={}", size);
        debug_assert!(
            size <= MAX_BIN_SIZE,
            "Alloc request for {} bytes is too big.",
            size
        );
        debug_assert!(align <= VM::MAX_ALIGNMENT);
        debug_assert!(align >= VM::MIN_ALIGNMENT);
        // debug_assert!(offset == 0);

        let block = self.find_free_block_local(size, align);
        let addr = self.block_alloc(block);

        #[cfg(debug_assertions)]
        if *self.plan.options().precise_stress && self.plan.base().is_stress_test_gc_enabled() {
            // If we are doing precise stress GC, we should not get any memory from fastpath.
            assert!(block.is_zero());
            assert!(addr.is_zero());
        }

        if addr.is_zero() {
            return self.alloc_slow(size, align, offset);
        }
        allocator::align_allocation::<VM>(addr, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // Try get a block from the space
        let block = self.acquire_fresh_block(size, align, false);
        if block.is_zero() {
            return Address::ZERO;
        }

        let addr = self.block_alloc(block);
        allocator::align_allocation::<VM>(addr, align, offset)
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        Block::BYTES
    }

    fn alloc_slow_once_precise_stress(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        need_poll: bool,
    ) -> Address {
        trace!("allow slow precise stress s={}", size);
        if need_poll {
            self.acquire_fresh_block(0, 0, true);
        }

        // mimic what fastpath allocation does, except that we allocate from available_blocks_stress.
        let block = self.find_free_block_stress(size, align);
        if block.is_zero() {
            return Address::ZERO;
        }
        let cell = self.block_alloc(block);
        allocator::align_allocation::<VM>(cell, align, offset)
    }

    // #[cfg(feature = "eager_sweeping")]
    // #[allow(unused_variables)]
    // fn alloc_slow_once_precise_stress(
    //     &mut self,
    //     size: usize,
    //     align: usize,
    //     offset: isize,
    //     need_poll: bool,
    // ) -> Address {
    //     let bin = mi_bin::<VM>(size, align) as usize;
    //     let consumed = self.consumed_blocks.get_mut(bin).unwrap();
    //     let available = self.available_blocks.get_mut(bin).unwrap();
    //     consumed.append(available);
    //     unsafe { Address::zero() }
    // }

    fn on_mutator_destroy(&mut self) {
        self.abandon_blocks();
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    // New free list allcoator
    pub fn new(
        tls: VMThread,
        space: &'static MarkSweepSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        FreeListAllocator {
            tls,
            space,
            plan,
            available_blocks: new_empty_block_lists(),
            available_blocks_stress: new_empty_block_lists(),
            unswept_blocks: new_empty_block_lists(),
            consumed_blocks: new_empty_block_lists(),
        }
    }

    // Find a free cell within a given block
    pub fn block_alloc(
        &mut self,
        block: Block,
        // size: usize,
        // align: usize,
        // offset: isize,
    ) -> Address {
        if block.is_zero() {
            return unsafe { Address::zero() };
        }
        let cell = block.load_free_list();
        if cell.is_zero() {
            // return self.alloc_slow(size, align, offset);
            return cell; // return failed allocation
        }
        let next_cell = unsafe { cell.load::<Address>() };
        // Clear the link
        unsafe { cell.store::<Address>(Address::ZERO) };
        debug_assert!(
            next_cell.is_zero() || block.includes_address(next_cell),
            "next_cell {} is not in {:?}",
            next_cell,
            block
        );
        block.store_free_list(next_cell);

        // Zeroing memory right before we return it.
        // If we move the zeroing to somewhere else, we need to clear the list link here: cell.store::<Address>(Address::ZERO)
        let cell_size = block.load_block_cell_size();
        crate::util::memory::zero(cell, cell_size);

        // Make sure the memory is zeroed. This looks silly as we zero the cell right before this check.
        // But we would need to move the zeroing to somewhere so we can do zeroing at a coarser grainularity.
        #[cfg(debug_assertions)]
        {
            let mut cursor = cell;
            while cursor < cell + cell_size {
                debug_assert_eq!(unsafe { cursor.load::<usize>() }, 0);
                cursor += crate::util::constants::BYTES_IN_ADDRESS;
            }
        }

        cell
    }

    // Find an available block when stress GC is enabled. This includes getting a block from the space.
    fn find_free_block_stress(&mut self, size: usize, align: usize) -> Block {
        let mut block = Self::find_free_block_with(
            &mut self.available_blocks_stress,
            &mut self.consumed_blocks,
            size,
            align,
        );
        if block.is_zero() {
            block = self.recycle_local_blocks(size, align, true);
        }
        if block.is_zero() {
            block = self.acquire_fresh_block(size, align, true);
        }
        block
    }

    // Find an available block from local block lists
    #[inline(always)]
    fn find_free_block_local(&mut self, size: usize, align: usize) -> Block {
        let block = Self::find_free_block_with(
            &mut self.available_blocks,
            &mut self.consumed_blocks,
            size,
            align,
        );
        if block.is_zero() {
            self.recycle_local_blocks(size, align, false)
        } else {
            block
        }
    }

    // Find an available block
    // This will usually be the first block on the available list. If all available blocks are found
    // to be full, other lists are searched
    // This function allows different available block lists -- normal allocation uses self.avaialble_blocks, and precise stress test uses self.avialable_blocks_stress.
    #[inline(always)]
    fn find_free_block_with(
        available_blocks: &mut BlockLists,
        consumed_blocks: &mut BlockLists,
        size: usize,
        align: usize,
    ) -> Block {
        let bin = mi_bin::<VM>(size, align);
        debug_assert!(bin <= MAX_BIN);

        let available = &mut available_blocks[bin];
        debug_assert!(available.size >= size);

        if !available.is_empty() {
            let mut block = available.first;

            while !block.is_zero() {
                if block.has_free_cells() {
                    return block;
                }
                available.pop();
                consumed_blocks.get_mut(bin).unwrap().push(block);

                block = available.first;
            }
        }

        debug_assert!(available_blocks[bin].is_empty());
        Block::ZERO_BLOCK
    }

    /// Add a block to the given bin in the available block lists. Depending on which available block list we are using, this
    /// method may add the block to available_blocks, or available_blocks_stress.
    #[inline(always)]
    fn add_to_available_blocks(&mut self, bin: usize, block: Block, stress: bool) {
        if stress {
            debug_assert!(self.plan.base().is_precise_stress());
            self.available_blocks_stress[bin].push(block);
        } else {
            self.available_blocks[bin].push(block);
        }
    }

    /// Tries to recycle local blocks if there is any. This is a no-op for eager sweeping mark sweep.
    #[cfg(not(feature = "eager_sweeping"))]
    #[inline]
    pub fn recycle_local_blocks(&mut self, size: usize, align: usize, _stress_test: bool) -> Block {
        // attempt to sweep
        loop {
            let bin = mi_bin::<VM>(size, align);
            debug_assert!(self.available_blocks[bin].is_empty()); // only use this function if there are no blocks available

            let block = self.unswept_blocks.get_mut(bin).unwrap().pop();
            if block.is_zero() {
                // no more blocks to sweep
                break;
            }
            block.sweep::<VM>();
            if block.has_free_cells() {
                // recyclable block
                self.add_to_available_blocks(
                    bin,
                    block,
                    self.plan.base().is_stress_test_gc_enabled(),
                );
                return block;
            } else {
                // nothing was freed from this block
                self.consumed_blocks.get_mut(bin).unwrap().push(block);
            }
        }
        Block::ZERO_BLOCK
    }

    /// Tries to recycle local blocks if there is any. This is a no-op for eager sweeping mark sweep.
    #[cfg(feature = "eager_sweeping")]
    #[inline]
    pub fn recycle_local_blocks(
        &mut self,
        _size: usize,
        _align: usize,
        _stress_test: bool,
    ) -> Block {
        Block::ZERO_BLOCK
    }

    /// Get a block from the space.
    pub fn acquire_fresh_block(&mut self, size: usize, align: usize, stress_test: bool) -> Block {
        // fresh block
        let bin = mi_bin::<VM>(size, align);
        loop {
            match self.space.acquire_block(self.tls, size, align) {
                crate::policy::marksweepspace::native_ms::BlockAcquireResult::Fresh(block) => {
                    if block.is_zero() {
                        // GC
                        return block;
                    }
                    self.add_to_available_blocks(bin, block, stress_test);
                    self.init_block(block, self.available_blocks[bin].size);

                    return block;
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::AbandonedAvailable(block) => {
                    block.store_tls(self.tls);
                    if block.has_free_cells() {
                        self.add_to_available_blocks(bin, block, stress_test);
                        return block;
                    } else {
                        self.consumed_blocks[bin].push(block);
                    }
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::AbandonedUnswept(block) => {
                    block.store_tls(self.tls);
                    block.sweep::<VM>();
                    if block.has_free_cells() {
                        self.add_to_available_blocks(bin, block, stress_test);
                        return block;
                    } else {
                        self.consumed_blocks[bin].push(block);
                    }
                }
            }
        }
    }

    pub fn init_block(&self, block: Block, cell_size: usize) {
        self.space.record_new_block(block);

        // construct free list
        let block_end = block.start() + Block::BYTES;
        let mut old_cell = unsafe { Address::zero() };
        let mut new_cell = block.start();

        let final_cell = loop {
            unsafe {
                new_cell.store::<Address>(old_cell);
            }
            old_cell = new_cell;
            new_cell += cell_size;
            if new_cell + cell_size > block_end {
                break old_cell;
            };
        };

        block.store_free_list(final_cell);
        // block.store_local_free_list(unsafe { Address::zero() });
        // block.store_thread_free_list(unsafe { Address::zero() });
        block.store_block_cell_size(cell_size);

        self.store_block_tls(block);
    }

    // alloc bit required for non GC context
    // pub fn sweep_block(&self, block: Block) {
    // let cell_size = block.load_block_cell_size::<VM>();
    // debug_assert!(cell_size != 0);
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
        block.store_tls(self.tls);
    }

    #[cfg(not(feature = "eager_sweeping"))]
    pub fn reset(&mut self) {
        trace!("reset");
        // consumed and available are now unswept
        let mut bin = 0;
        while bin < MAX_BIN + 1 {
            let unswept = self.unswept_blocks.get_mut(bin).unwrap();
            unswept.lock();

            let mut sweep_later = |list: &mut BlockList| {
                list.lock();
                unswept.append(list);
                list.unlock();
            };

            sweep_later(&mut self.available_blocks[bin]);
            sweep_later(&mut self.available_blocks_stress[bin]);
            sweep_later(&mut self.consumed_blocks[bin]);

            unswept.unlock();
            bin += 1;
        }

        if self.plan.base().is_precise_stress() && self.plan.base().is_stress_test_gc_enabled() {
            self.abandon_blocks();
        }
    }

    #[cfg(feature = "eager_sweeping")]
    pub fn reset(&mut self) {
        debug!("reset");
        // sweep all blocks and push consumed onto available list
        let mut bin = 0;
        while bin < MAX_BIN + 1 {
            let sweep = |first_block: Block, used_blocks: bool| {
                let mut cursor = first_block;
                while !cursor.is_zero() {
                    if used_blocks {
                        cursor.sweep::<VM>();
                        cursor = cursor.load_next_block();
                    } else {
                        let next = cursor.load_next_block();
                        if !cursor.attempt_release(self.space) {
                            cursor.sweep::<VM>();
                        }
                        cursor = next;
                    }
                }
            };

            sweep(self.available_blocks[bin].first, true);
            sweep(self.available_blocks_stress[bin].first, true);

            // Sweep consumed blocks, and also push the blocks back to the available list.
            sweep(self.consumed_blocks[bin].first, false);
            if self.plan.base().is_precise_stress() && self.plan.base().is_stress_test_gc_enabled()
            {
                debug_assert!(self.plan.base().is_precise_stress());
                self.available_blocks_stress[bin].append(&mut self.consumed_blocks[bin]);
            } else {
                self.available_blocks[bin].append(&mut self.consumed_blocks[bin]);
            }

            bin += 1;
        }
    }

    pub fn abandon_blocks(&mut self) {
        let mut abandoned = self.space.abandoned_available.lock().unwrap();
        let mut abandoned_consumed = self.space.abandoned_consumed.lock().unwrap();
        let mut abandoned_unswept = self.space.abandoned_unswept.lock().unwrap();
        let mut i = 0;
        while i < MI_BIN_FULL {
            let available = self.available_blocks.get_mut(i).unwrap();
            if !available.is_empty() {
                abandoned[i].append(available);
            }

            let available_stress = self.available_blocks_stress.get_mut(i).unwrap();
            if !available_stress.is_empty() {
                abandoned[i].append(available_stress);
            }

            let consumed = self.consumed_blocks.get_mut(i).unwrap();
            if !consumed.is_empty() {
                abandoned_consumed[i].append(consumed);
            }

            let unswept = self.unswept_blocks.get_mut(i).unwrap();
            if !unswept.is_empty() {
                abandoned_unswept[i].append(unswept);
            }
            i += 1;
        }
    }
}

/// Align a byte size to a size in machine words
/// i.e. byte size == `wsize*sizeof(void*)`
/// adapted from _mi_wsize_from_size in mimalloc
fn mi_wsize_from_size(size: usize) -> usize {
    (size + MI_INTPTR_SIZE - 1) / MI_INTPTR_SIZE
}

pub fn mi_bin<VM: VMBinding>(size: usize, align: usize) -> usize {
    let size = allocator::get_maximum_aligned_size::<VM>(size, align);
    mi_bin_from_size(size)
}

fn mi_bin_from_size(size: usize) -> usize {
    // adapted from _mi_bin in mimalloc
    let mut wsize: usize = mi_wsize_from_size(size);
    debug_assert!(wsize <= MI_LARGE_OBJ_WSIZE_MAX);
    let bin: u8;
    if wsize <= 1 {
        bin = 1;
    } else if wsize <= 8 {
        bin = wsize as u8;
        // bin = ((wsize + 1) & !1) as u8; // round to double word sizes
    } else {
        wsize -= 1;
        let b = (MI_INTPTR_BITS - 1 - usize::leading_zeros(wsize) as usize) as u8; // note: wsize != 0
        bin = ((b << 2) + ((wsize >> (b - 2)) & 0x03) as u8) - 3;
    }
    bin as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_bin_size_range(bin: usize, bins: &BlockLists) -> Option<(usize, usize)> {
        if bin == 0 || bin > MAX_BIN {
            None
        } else if bin == 1 {
            Some((0, bins[1].size))
        } else {
            Some((bins[bin - 1].size, bins[bin].size))
        }
    }

    #[test]
    fn test_mi_bin() {
        let block_lists = new_empty_block_lists();
        for size in 0..=MAX_BIN_SIZE {
            let bin = mi_bin_from_size(size);
            let bin_range = get_bin_size_range(bin, &block_lists);
            assert!(bin_range.is_some(), "Invalid bin {} for size {}", bin, size);
            assert!(
                size >= bin_range.unwrap().0 && bin < bin_range.unwrap().1,
                "Assigning size={} to bin={} ({:?}) incorrect",
                size,
                bin,
                bin_range.unwrap()
            );
        }
    }
}
