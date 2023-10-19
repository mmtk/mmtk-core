// This is a free list allocator written based on Microsoft's mimalloc allocator https://www.microsoft.com/en-us/research/publication/mimalloc-free-list-sharding-in-action/

use std::sync::Arc;

use crate::policy::marksweepspace::native_ms::*;
use crate::util::alloc::allocator;
use crate::util::alloc::Allocator;
use crate::util::linear_scan::Region;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;

use super::allocator::AllocatorContext;

/// A MiMalloc free list allocator
#[repr(C)]
pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    context: Arc<AllocatorContext<VM>>,
    /// blocks with free space
    pub available_blocks: BlockLists,
    /// blocks with free space for precise stress GC
    /// For precise stress GC, we need to be able to trigger slowpath allocation for
    /// each allocation. To achieve this, we put available blocks to this list. So
    /// normal fastpath allocation will fail, as they will see the block lists
    /// as empty.
    pub available_blocks_stress: BlockLists,
    /// blocks that are marked, not swept
    pub unswept_blocks: BlockLists,
    /// full blocks
    pub consumed_blocks: BlockLists,
}

impl<VM: VMBinding> Allocator<VM> for FreeListAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_space(&self) -> &'static dyn crate::policy::space::Space<VM> {
        self.space
    }

    fn get_context(&self) -> &AllocatorContext<VM> {
        &self.context
    }

    // Find a block with free space and allocate to it
    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        debug_assert!(
            size <= MAX_BIN_SIZE,
            "Alloc request for {} bytes is too big.",
            size
        );
        debug_assert!(align <= VM::MAX_ALIGNMENT);
        debug_assert!(align >= VM::MIN_ALIGNMENT);

        if let Some(block) = self.find_free_block_local(size, align) {
            let cell = self.block_alloc(block);
            if !cell.is_zero() {
                // We succeeded in fastpath alloc, this cannot be precise stress test
                debug_assert!(
                    !(*self.context.options.precise_stress
                        && self.context.options.is_stress_test_gc_enabled())
                );

                let res = allocator::align_allocation::<VM>(cell, align, offset);
                // Make sure that the allocation region is within the cell
                #[cfg(debug_assertions)]
                {
                    let cell_size = block.load_block_cell_size();
                    debug_assert!(
                        res + size <= cell + cell_size,
                        "Allocating (size = {}, align = {}, offset = {}) to the cell {} of size {}, but the end of the allocation region {} is beyond the cell end {}",
                        size, align, offset, cell, cell_size, res + size, cell + cell_size
                    );
                }
                return res;
            }
        }

        self.alloc_slow(size, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: usize) -> Address {
        // Try get a block from the space
        if let Some(block) = self.acquire_global_block(size, align, false) {
            let addr = self.block_alloc(block);
            allocator::align_allocation::<VM>(addr, align, offset)
        } else {
            Address::ZERO
        }
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
        offset: usize,
        need_poll: bool,
    ) -> Address {
        trace!("allow slow precise stress s={}", size);
        if need_poll {
            self.acquire_global_block(0, 0, true);
        }

        // mimic what fastpath allocation does, except that we allocate from available_blocks_stress.
        if let Some(block) = self.find_free_block_stress(size, align) {
            let cell = self.block_alloc(block);
            allocator::align_allocation::<VM>(cell, align, offset)
        } else {
            Address::ZERO
        }
    }

    fn on_mutator_destroy(&mut self) {
        self.abandon_blocks();
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    // New free list allcoator
    pub(crate) fn new(
        tls: VMThread,
        space: &'static MarkSweepSpace<VM>,
        context: Arc<AllocatorContext<VM>>,
    ) -> Self {
        FreeListAllocator {
            tls,
            space,
            context,
            available_blocks: new_empty_block_lists(),
            available_blocks_stress: new_empty_block_lists(),
            unswept_blocks: new_empty_block_lists(),
            consumed_blocks: new_empty_block_lists(),
        }
    }

    // Find a free cell within a given block
    fn block_alloc(&mut self, block: Block) -> Address {
        let cell = block.load_free_list();
        if cell.is_zero() {
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
    fn find_free_block_stress(&mut self, size: usize, align: usize) -> Option<Block> {
        Self::find_free_block_with(
            &mut self.available_blocks_stress,
            &mut self.consumed_blocks,
            size,
            align,
        )
        .or_else(|| self.recycle_local_blocks(size, align, true))
        .or_else(|| self.acquire_global_block(size, align, true))
    }

    // Find an available block from local block lists
    fn find_free_block_local(&mut self, size: usize, align: usize) -> Option<Block> {
        Self::find_free_block_with(
            &mut self.available_blocks,
            &mut self.consumed_blocks,
            size,
            align,
        )
        .or_else(|| self.recycle_local_blocks(size, align, false))
    }

    // Find an available block
    // This will usually be the first block on the available list. If all available blocks are found
    // to be full, other lists are searched
    // This function allows different available block lists -- normal allocation uses self.avaialble_blocks, and precise stress test uses self.avialable_blocks_stress.
    fn find_free_block_with(
        available_blocks: &mut BlockLists,
        consumed_blocks: &mut BlockLists,
        size: usize,
        align: usize,
    ) -> Option<Block> {
        let bin = mi_bin::<VM>(size, align);
        debug_assert!(bin <= MAX_BIN);

        let available = &mut available_blocks[bin];
        debug_assert!(available.size >= size);

        if !available.is_empty() {
            let mut cursor = available.first;

            while let Some(block) = cursor {
                if block.has_free_cells() {
                    return Some(block);
                }
                available.pop();
                consumed_blocks.get_mut(bin).unwrap().push(block);

                cursor = available.first;
            }
        }

        debug_assert!(available_blocks[bin].is_empty());
        None
    }

    /// Add a block to the given bin in the available block lists. Depending on which available block list we are using, this
    /// method may add the block to available_blocks, or available_blocks_stress.
    fn add_to_available_blocks(&mut self, bin: usize, block: Block, stress: bool) {
        if stress {
            debug_assert!(*self.context.options.precise_stress);
            self.available_blocks_stress[bin].push(block);
        } else {
            self.available_blocks[bin].push(block);
        }
    }

    /// Tries to recycle local blocks if there is any. This is a no-op for eager sweeping mark sweep.
    fn recycle_local_blocks(
        &mut self,
        size: usize,
        align: usize,
        _stress_test: bool,
    ) -> Option<Block> {
        if cfg!(feature = "eager_sweeping") {
            // We have swept blocks in the last GC. If we run out of available blocks, there is nothing we can do.
            None
        } else {
            // Get blocks from unswept_blocks and attempt to sweep
            loop {
                let bin = mi_bin::<VM>(size, align);
                debug_assert!(self.available_blocks[bin].is_empty()); // only use this function if there are no blocks available

                if let Some(block) = self.unswept_blocks.get_mut(bin).unwrap().pop() {
                    block.sweep::<VM>();
                    if block.has_free_cells() {
                        // recyclable block
                        self.add_to_available_blocks(
                            bin,
                            block,
                            self.context.options.is_stress_test_gc_enabled(),
                        );
                        return Some(block);
                    } else {
                        // nothing was freed from this block
                        self.consumed_blocks.get_mut(bin).unwrap().push(block);
                    }
                } else {
                    return None;
                }
            }
        }
    }

    /// Get a block from the space.
    fn acquire_global_block(
        &mut self,
        size: usize,
        align: usize,
        stress_test: bool,
    ) -> Option<Block> {
        let bin = mi_bin::<VM>(size, align);
        loop {
            match self.space.acquire_block(self.tls, size, align) {
                crate::policy::marksweepspace::native_ms::BlockAcquireResult::Exhausted => {
                    // GC
                    return None;
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::Fresh(block) => {
                    self.add_to_available_blocks(bin, block, stress_test);
                    self.init_block(block, self.available_blocks[bin].size);

                    return Some(block);
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::AbandonedAvailable(block) => {
                    block.store_tls(self.tls);
                    if block.has_free_cells() {
                        self.add_to_available_blocks(bin, block, stress_test);
                        return Some(block);
                    } else {
                        self.consumed_blocks[bin].push(block);
                    }
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::AbandonedUnswept(block) => {
                    block.store_tls(self.tls);
                    block.sweep::<VM>();
                    if block.has_free_cells() {
                        self.add_to_available_blocks(bin, block, stress_test);
                        return Some(block);
                    } else {
                        self.consumed_blocks[bin].push(block);
                    }
                }
            }
        }
    }

    fn init_block(&self, block: Block, cell_size: usize) {
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
        block.store_block_cell_size(cell_size);
        #[cfg(feature = "malloc_native_mimalloc")]
        {
            block.store_local_free_list(Address::ZERO);
            block.store_thread_free_list(Address::ZERO);
        }

        self.store_block_tls(block);
    }

    #[cfg(feature = "malloc_native_mimalloc")]
    fn free(&self, addr: Address) {
        use crate::util::ObjectReference;
        let block = Block::from_unaligned_address(addr);
        let block_tls = block.load_tls();

        if self.tls == block_tls {
            // same thread that allocated
            let local_free = block.load_local_free_list();
            unsafe {
                addr.store(local_free);
            }
            block.store_local_free_list(addr);
        } else {
            // different thread to allocator
            unreachable!(
                "tlss don't match freeing from block {}, my tls = {:?}, block tls = {:?}",
                block.start(),
                self.tls,
                block.load_tls()
            );

            // I am not sure whether the following code would be used to free a block for other thread. I will just keep it here as commented out.
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
        unsafe {
            crate::util::metadata::vo_bit::unset_vo_bit_unsafe::<VM>(
                ObjectReference::from_raw_address(addr),
            )
        };
    }

    fn store_block_tls(&self, block: Block) {
        block.store_tls(self.tls);
    }

    pub(crate) fn prepare(&mut self) {
        // For lazy sweeping, it doesn't matter whether we do it in prepare or release.
        // However, in the release phase, we will do block-level sweeping. And that will cause
        // race if we also reset the allocator in release (which will mutate on the block lists).
        // So we just move reset to the prepare phase.
        #[cfg(not(feature = "eager_sweeping"))]
        self.reset();
    }

    pub(crate) fn release(&mut self) {
        // For eager sweeping, we have to do this in the release phase when we know the liveness of the blocks
        #[cfg(feature = "eager_sweeping")]
        self.reset();
    }

    /// Do we abandon allocator local blocks in reset?
    /// We should do this for GC. Otherwise, blocks will be held by each allocator, and they cannot
    /// be reused by other allocators. This is measured to cause up to 100% increase of the min heap size
    /// for mark sweep.
    const ABANDON_BLOCKS_IN_RESET: bool = true;

    #[cfg(not(feature = "eager_sweeping"))]
    fn reset(&mut self) {
        trace!("reset");
        // consumed and available are now unswept
        for bin in 0..MI_BIN_FULL {
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
        }

        if Self::ABANDON_BLOCKS_IN_RESET {
            self.abandon_blocks();
        }
    }

    #[cfg(feature = "eager_sweeping")]
    fn reset(&mut self) {
        debug!("reset");
        // sweep all blocks and push consumed onto available list
        for bin in 0..MI_BIN_FULL {
            // Sweep available blocks
            self.available_blocks[bin].sweep_blocks(self.space);
            self.available_blocks_stress[bin].sweep_blocks(self.space);

            // Sweep consumed blocks, and also push the blocks back to the available list.
            self.consumed_blocks[bin].sweep_blocks(self.space);
            if *self.context.options.precise_stress
                && self.context.options.is_stress_test_gc_enabled()
            {
                debug_assert!(*self.context.options.precise_stress);
                self.available_blocks_stress[bin].append(&mut self.consumed_blocks[bin]);
            } else {
                self.available_blocks[bin].append(&mut self.consumed_blocks[bin]);
            }

            // For eager sweeping, we should not have unswept blocks
            assert!(self.unswept_blocks[bin].is_empty());
        }

        if Self::ABANDON_BLOCKS_IN_RESET {
            self.abandon_blocks();
        }
    }

    fn abandon_blocks(&mut self) {
        let mut abandoned = self.space.abandoned.lock().unwrap();
        for i in 0..MI_BIN_FULL {
            let available = self.available_blocks.get_mut(i).unwrap();
            if !available.is_empty() {
                abandoned.available[i].append(available);
            }

            let available_stress = self.available_blocks_stress.get_mut(i).unwrap();
            if !available_stress.is_empty() {
                abandoned.available[i].append(available_stress);
            }

            let consumed = self.consumed_blocks.get_mut(i).unwrap();
            if !consumed.is_empty() {
                abandoned.consumed[i].append(consumed);
            }

            let unswept = self.unswept_blocks.get_mut(i).unwrap();
            if !unswept.is_empty() {
                abandoned.unswept[i].append(unswept);
            }
        }
    }
}
