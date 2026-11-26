// This is a free list allocator written based on Microsoft's mimalloc allocator https://www.microsoft.com/en-us/research/publication/mimalloc-free-list-sharding-in-action/

use std::sync::Arc;

use super::allocator::AllocatorContext;
use crate::policy::marksweepspace::native_ms::*;
use crate::util::alloc::allocator;
use crate::util::alloc::Allocator;
use crate::util::linear_scan::Region;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;

/// The bin and cell size [`FreeListAllocator`] uses for an allocation of `size` bytes.
pub fn free_list_size_class<VM: VMBinding>(size: usize, align: usize) -> (usize, usize) {
    size_class::<VM>(size, align)
}

/// A MiMalloc free list allocator
///
/// Free cells are kept as intervals of adjacent cells. Each bin bumps through one interval,
/// `[cursors[bin], limits[bin])`. See [`crate::util::alloc::AllocatorInfo::FreeList`].
#[repr(C)]
pub struct FreeListAllocator<VM: VMBinding> {
    /// [`VMThread`] associated with this allocator instance
    pub tls: VMThread,
    /// Per bin: the next free cell of the current interval. Zero when the bin has no interval.
    pub cursors: [Address; MI_BIN_FULL],
    /// Per bin: the end of the current interval.
    pub limits: [Address; MI_BIN_FULL],
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

    // Bump through the bin's current interval, or take the next interval and bump through that.
    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        debug_assert!(
            size <= MAX_BIN_SIZE,
            "Alloc request for {} bytes is too big.",
            size
        );
        debug_assert!(align <= VM::MAX_ALIGNMENT);
        debug_assert!(align >= VM::MIN_ALIGNMENT);

        let (bin, cell_size) = size_class::<VM>(size, align);
        let cell = self.cursors[bin];
        if cell + cell_size <= self.limits[bin] {
            self.cursors[bin] = cell + cell_size;
            crate::util::memory::zero(cell, cell_size);
            return self.finish_alloc(cell, cell_size, size, align, offset);
        }

        if let Some(block) = self.find_free_block_local(size, align) {
            if let Some(cell) = self.take_interval(block, bin, false) {
                // We succeeded in fastpath alloc, this cannot be precise stress test
                debug_assert!(
                    !(*self.context.options.precise_stress
                        && self.context.options.is_stress_test_gc_enabled())
                );
                return self.finish_alloc(cell, cell_size, size, align, offset);
            }
        }

        self.alloc_slow(size, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: usize) -> Address {
        // Try get a block from the space
        if let Some(block) = self.acquire_global_block(size, align, false) {
            let bin = mi_bin::<VM>(size, align);
            let addr = self
                .take_interval(block, bin, false)
                .unwrap_or(Address::ZERO);
            if addr.is_zero() {
                return addr;
            }
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
            // Take one cell only, so that the next allocation comes here again.
            let bin = mi_bin::<VM>(size, align);
            let Some(cell) = self.take_interval(block, bin, true) else {
                return Address::ZERO;
            };
            allocator::align_allocation::<VM>(cell, align, offset)
        } else {
            Address::ZERO
        }
    }

    fn on_mutator_destroy(&mut self) {
        self.return_intervals();
        let mut global = self.space.get_abandoned_block_lists().lock().unwrap();
        self.abandon_blocks(&mut global);
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
            cursors: [Address::ZERO; MI_BIN_FULL],
            limits: [Address::ZERO; MI_BIN_FULL],
            space,
            context,
            available_blocks: new_empty_block_lists(),
            available_blocks_stress: new_empty_block_lists(),
            unswept_blocks: new_empty_block_lists(),
            consumed_blocks: new_empty_block_lists(),
        }
    }

    /// Align the allocation in the cell just taken and check it fits.
    #[inline(always)]
    fn finish_alloc(
        &self,
        cell: Address,
        cell_size: usize,
        size: usize,
        align: usize,
        offset: usize,
    ) -> Address {
        let res = allocator::align_allocation::<VM>(cell, align, offset);
        debug_assert!(
            res + size <= cell + cell_size,
            "Allocating (size = {}, align = {}, offset = {}) to the cell {} of size {}, but the end of the allocation region {} is beyond the cell end {}",
            size, align, offset, cell, cell_size, res + size, cell + cell_size
        );
        res
    }

    /// Take the next interval from `block` and return its zeroed first cell. The rest becomes the
    /// bin's bump region, or goes back to the block if `one_cell` is set (precise stress GC).
    fn take_interval(&mut self, block: Block, bin: usize, one_cell: bool) -> Option<Address> {
        let (start, end) = block.pop_interval()?;
        let cell_size = block.load_block_cell_size();
        debug_assert_eq!(
            cell_size,
            crate::policy::marksweepspace::native_ms::BIN_SIZES[bin]
        );
        let end = if one_cell && start + cell_size < end {
            block.push_interval(start + cell_size, end);
            start + cell_size
        } else {
            end
        };
        crate::util::memory::zero(start, cell_size);
        if one_cell {
            debug_assert_eq!(end, start + cell_size);
        } else {
            self.return_interval(bin);
            self.cursors[bin] = start + cell_size;
            self.limits[bin] = end;
        }
        Some(start)
    }

    /// Give the unused part of `bin`'s bump region back to its block.
    fn return_interval(&mut self, bin: usize) {
        let (cursor, limit) = (self.cursors[bin], self.limits[bin]);
        self.cursors[bin] = Address::ZERO;
        self.limits[bin] = Address::ZERO;
        if cursor < limit {
            Block::from_unaligned_address(cursor).push_interval(cursor, limit);
        }
    }

    /// Give every bump region back to its block. Called at the start of a GC's prepare and
    /// release, so that sweeping, block release and allocate-black see every free cell.
    fn return_intervals(&mut self) {
        for bin in 0..MI_BIN_FULL {
            self.return_interval(bin);
        }
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

                let block = self.unswept_blocks.get_mut(bin).unwrap().pop()?;

                block.sweep(self.space);
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
            match self.space.acquire_block(self.tls, size, align, self.get_context().get_alloc_options()) {
                crate::policy::marksweepspace::native_ms::BlockAcquireResult::Exhausted => {
                    debug!("Acquire global block: None");
                    // GC
                    return None;
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::Fresh(block) => {
                    debug!("Acquire global block: Fresh {:?}", block);
                    self.add_to_available_blocks(bin, block, stress_test);
                    self.init_block(block, self.available_blocks[bin].size);

                    return Some(block);
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::AbandonedAvailable(block) => {
                    debug!("Acquire global block: AbandonedAvailable {:?}", block);
                    block.store_tls(self.tls);
                    if block.has_free_cells() {
                        self.add_to_available_blocks(bin, block, stress_test);
                        return Some(block);
                    } else {
                        self.consumed_blocks[bin].push(block);
                    }
                }

                crate::policy::marksweepspace::native_ms::BlockAcquireResult::AbandonedUnswept(block) => {
                    debug!("Acquire global block: AbandonedUnswep {:?}", block);
                    block.store_tls(self.tls);
                    block.sweep(self.space);
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
        debug_assert_ne!(cell_size, 0);
        self.space.record_new_block(block);

        // The whole block is one interval of free cells.
        let cells = Block::BYTES / cell_size;
        let mut intervals = IntervalListBuilder::new(cell_size, self.space.coalesce_free_cells());
        if cells > 0 {
            // Adding the first and the last cell spans the interval; cells in between are implied.
            intervals.add_interval(block.start(), block.start() + cells * cell_size);
        }
        block.store_free_list(intervals.finish());
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
        assert!(!addr.is_zero(), "Attempted to free zero address.");

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
        // Note: We cannot use `unset_vo_bit_unsafe` because two threads may attempt to free
        // objects at adjacent addresses, and they may share the same byte in the VO bit metadata.
        crate::util::metadata::vo_bit::unset_vo_bit(unsafe {
            ObjectReference::from_raw_address_unchecked(addr)
        })
    }

    fn store_block_tls(&self, block: Block) {
        block.store_tls(self.tls);
    }

    pub(crate) fn prepare(&mut self) {
        self.return_intervals();
    }

    pub(crate) fn release(&mut self) {
        self.return_intervals();
        for bin in 0..MI_BIN_FULL {
            let unswept = self.unswept_blocks.get_mut(bin).unwrap();

            // If we do eager sweeping, we should have no unswept blocks.
            #[cfg(feature = "eager_sweeping")]
            debug_assert!(unswept.is_empty());

            let mut sweep_later = |list: &mut BlockList| {
                list.release_blocks(self.space);

                // For eager sweeping, that's it.  We just release unmarked blocks, and leave marked
                // blocks to be swept later in the `SweepChunk` work packet.

                // For lazy sweeping, we move blocks from available and consumed to unswept.  When
                // an allocator tries to use them, they will sweep the block.
                if cfg!(not(feature = "eager_sweeping")) {
                    unswept.append(list);
                }
            };

            sweep_later(&mut self.available_blocks[bin]);
            sweep_later(&mut self.available_blocks_stress[bin]);
            sweep_later(&mut self.consumed_blocks[bin]);
        }

        // We abandon block lists immediately.  Otherwise, some mutators will hold lots of blocks
        // locally and prevent other mutators to use.
        {
            let mut global = self.space.get_abandoned_block_lists_in_gc().lock().unwrap();
            self.abandon_blocks(&mut global);
        }

        self.space.release_packet_done();
    }

    fn abandon_blocks(&mut self, global: &mut AbandonedBlockLists) {
        for i in 0..MI_BIN_FULL {
            let available = self.available_blocks.get_mut(i).unwrap();
            if !available.is_empty() {
                global.available[i].append(available);
            }

            let available_stress = self.available_blocks_stress.get_mut(i).unwrap();
            if !available_stress.is_empty() {
                global.available[i].append(available_stress);
            }

            let consumed = self.consumed_blocks.get_mut(i).unwrap();
            if !consumed.is_empty() {
                global.consumed[i].append(consumed);
            }

            let unswept = self.unswept_blocks.get_mut(i).unwrap();
            if !unswept.is_empty() {
                global.unswept[i].append(unswept);
            }
        }
    }
}
