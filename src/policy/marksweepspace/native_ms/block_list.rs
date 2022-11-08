use super::Block;
use crate::util::alloc::allocator;
use crate::util::linear_scan::Region;
use crate::vm::VMBinding;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

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
    pub fn push(&mut self, block: Block) {
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
pub(crate) const MAX_BIN_SIZE: usize = 8192 * MI_INTPTR_SIZE;

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

/// Returns how many pages the block lists uses.
#[allow(unused)]
pub(crate) fn pages_used_by_blocklists(lists: &BlockLists) -> usize {
    let mut pages = 0;
    for bin in 1..=MAX_BIN {
        let list = &lists[bin];

        // walk the blocks
        let mut block = list.first;
        while !block.is_zero() {
            pages += Block::BYTES >> crate::util::constants::LOG_BYTES_IN_PAGE;
            block = block.load_next_block();
        }
    }

    pages
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
