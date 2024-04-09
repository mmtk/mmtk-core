use super::Block;
use crate::util::alloc::allocator;
use crate::util::linear_scan::Region;
use crate::vm::VMBinding;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
#[cfg(feature = "ms_block_list_sanity")]
use std::sync::Mutex;

/// List of blocks owned by the allocator
#[repr(C)]
pub struct BlockList {
    first: Option<Block>,
    last: Option<Block>,
    size: usize,
    lock: AtomicBool,
    #[cfg(feature = "ms_block_list_sanity")]
    sanity_list: Mutex<Vec<Block>>,
}

impl std::fmt::Debug for BlockList {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self.iter().collect::<Vec<Block>>())
    }
}

impl BlockList {
    const fn new(size: usize) -> BlockList {
        BlockList {
            first: None,
            last: None,
            size,
            lock: AtomicBool::new(false),
            #[cfg(feature = "ms_block_list_sanity")]
            sanity_list: Mutex::new(vec![]),
        }
    }

    // fn access_block_list<R: Copy, F: FnOnce() -> R>(&self, access_func: F) -> R {
    //     #[cfg(feature = "ms_block_list_sanity")]
    //     let mut sanity_list = self.sanity_list.lock().unwrap();

    //     let ret = access_func();

    //     // Verify the block list is the same as the sanity list
    //     #[cfg(feature = "ms_block_list_sanity")]
    //     {
    //         if !sanity_list.iter().map(|b| *b).eq(BlockListIterator { cursor: self.first }) {
    //             eprintln!("Sanity block list: {:?}", &mut sanity_list as &mut Vec<Block>);
    //             eprintln!("Actual block list: {:?}", self);
    //             panic!("Incorrect block list");
    //         }
    //     }

    //     ret
    // }
    #[cfg(feature = "ms_block_list_sanity")]
    fn verify_block_list(&self, sanity_list: &mut Vec<Block>) {
        if !sanity_list
            .iter()
            .map(|b| *b)
            .eq(BlockListIterator { cursor: self.first })
        {
            eprintln!("Sanity block list: {:?}", sanity_list);
            eprintln!("First {:?}", sanity_list.get(0));
            eprintln!("Actual block list: {:?}", self);
            eprintln!("First {:?}", self.first);
            eprintln!("Block list {:?}", self as *const _);
            panic!("Incorrect block list");
        }
    }

    /// List has no blocks
    pub fn is_empty(&self) -> bool {
        let ret = self.first.is_none();

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            self.verify_block_list(&mut sanity_list);
            assert_eq!(sanity_list.is_empty(), ret);
        }

        ret
    }

    /// Remove a block from the list
    pub fn remove(&mut self, block: Block) {
        trace!("Blocklist {:?}: Remove {:?}", self as *const _, block);
        match (block.load_prev_block(), block.load_next_block()) {
            (None, None) => {
                self.first = None;
                self.last = None;
            }
            (None, Some(next)) => {
                next.clear_prev_block();
                self.first = Some(next);
                // next.store_block_list(self);
                debug_assert_eq!(next.load_block_list(), self as *mut _);
            }
            (Some(prev), None) => {
                prev.clear_next_block();
                self.last = Some(prev);
                // prev.store_block_list(self);
                debug_assert_eq!(prev.load_block_list(), self as *mut _);
            }
            (Some(prev), Some(next)) => {
                prev.store_next_block(next);
                next.store_prev_block(prev);
            }
        }

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            if let Some((index, _)) = sanity_list
                .iter()
                .enumerate()
                .find(|&(_, &val)| val == block)
            {
                sanity_list.remove(index);
            } else {
                panic!("Cannot find {:?} in the block list", block);
            }
            self.verify_block_list(&mut sanity_list);
        }
    }

    /// Pop the first block in the list
    pub fn pop(&mut self) -> Option<Block> {
        let ret = if let Some(head) = self.first {
            if let Some(next) = head.load_next_block() {
                self.first = Some(next);
                next.clear_prev_block();
                next.store_block_list(self);
            } else {
                self.first = None;
                self.last = None;
            }
            head.clear_next_block();
            head.clear_prev_block();
            Some(head)
        } else {
            None
        };

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            let sanity_ret = if sanity_list.is_empty() {
                None
            } else {
                Some(sanity_list.remove(0)) // pop first
            };
            self.verify_block_list(&mut sanity_list);
            assert_eq!(sanity_ret, ret);
        }

        trace!("Blocklist {:?}: Pop = {:?}", self as *const _, ret);
        ret
    }

    /// Push block to the front of the list
    pub fn push(&mut self, block: Block) {
        trace!("Blocklist {:?}: Push {:?}", self as *const _, block);
        if self.is_empty() {
            block.clear_next_block();
            block.clear_prev_block();
            self.first = Some(block);
            self.last = Some(block);
        } else {
            let self_head = self.first.unwrap();
            block.store_next_block(self_head);
            self_head.store_prev_block(block);
            block.clear_prev_block();
            self.first = Some(block);
        }
        block.store_block_list(self);

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            sanity_list.insert(0, block); // push front
            self.verify_block_list(&mut sanity_list);
        }
    }

    /// Moves all the blocks of `other` into `self`, leaving `other` empty.
    pub fn append(&mut self, other: &mut BlockList) {
        trace!(
            "Blocklist {:?}: Append Blocklist {:?}",
            self as *const _,
            other as *const _
        );
        #[cfg(feature = "ms_block_list_sanity")]
        {
            // Check before merging
            let mut sanity_list = self.sanity_list.lock().unwrap();
            self.verify_block_list(&mut sanity_list);
            let mut sanity_list_other = other.sanity_list.lock().unwrap();
            other.verify_block_list(&mut sanity_list_other);
        }
        #[cfg(feature = "ms_block_list_sanity")]
        let mut sanity_list_in_other = other.sanity_list.lock().unwrap().clone();

        debug_assert_eq!(self.size, other.size);
        if !other.is_empty() {
            debug_assert!(
                other.first.unwrap().load_prev_block().is_none(),
                "The other list's head has prev block: prev{} -> head{}",
                other.first.unwrap().load_prev_block().unwrap().start(),
                other.first.unwrap().start()
            );
            if self.is_empty() {
                self.first = other.first;
                self.last = other.last;
            } else {
                debug_assert!(
                    self.first.unwrap().load_prev_block().is_none(),
                    "Current list's head has prev block: prev{} -> head{}",
                    self.first.unwrap().load_prev_block().unwrap().start(),
                    self.first.unwrap().start()
                );
                let self_tail = self.last.unwrap();
                let other_head = other.first.unwrap();
                self_tail.store_next_block(other_head);
                other_head.store_prev_block(self_tail);
                self.last = other.last;
            }
            let mut cursor = other.first;
            while let Some(block) = cursor {
                block.store_block_list(self);
                cursor = block.load_next_block();
            }
            other.reset();
        }

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            sanity_list.append(&mut sanity_list_in_other);
            self.verify_block_list(&mut sanity_list);
        }
    }

    /// Remove all blocks
    fn reset(&mut self) {
        trace!("Blocklist {:?}: Reset", self as *const _);
        self.first = None;
        self.last = None;

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            sanity_list.clear();
        }
    }

    /// Lock the list. The MiMalloc allocator mostly uses thread-local block lists, and those operations on the list
    /// do not need synchronisation. However, in cases where a block list may be accessed by multiple threads, we need
    /// to lock the list before accessing it.
    ///
    /// Our current sole use for locking is parallel sweeping. During the Release phase, multiple GC worker threads can
    /// sweep chunks and release mutators at the same time, and the same `BlockList` can be reached by traversing blocks in a chunk,
    /// and also by traversing blocks held by a mutator.  This lock is necessary to prevent
    /// multiple GC workers from mutating the same `BlockList` instance.
    pub fn lock(&mut self) {
        let mut success = false;
        while !success {
            success = self
                .lock
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
        }
        trace!("Blocklist {:?}: locked", self as *const _);
    }

    /// Unlock list. See the comments on the lock method.
    pub fn unlock(&mut self) {
        trace!("Blocklist {:?}: unlock", self as *const _);
        self.lock.store(false, Ordering::SeqCst);
    }

    /// Get an iterator for the block list.
    pub fn iter(&self) -> BlockListIterator {
        BlockListIterator { cursor: self.first }
    }

    /// Sweep all the blocks in the block list.
    pub fn sweep_blocks<VM: VMBinding>(&self, space: &super::MarkSweepSpace<VM>) {
        for block in self.iter() {
            if !block.attempt_release(space) {
                block.sweep::<VM>();
            }
        }
    }

    /// Get the size of this block list.
    pub fn size(&self) -> usize {
        let ret = self.size;

        #[cfg(feature = "ms_block_list_sanity")]
        {
            let mut sanity_list = self.sanity_list.lock().unwrap();
            self.verify_block_list(&mut sanity_list);
        }

        ret
    }

    /// Get the first block in the list.
    pub fn first(&self) -> Option<Block> {
        self.first
    }
}

pub struct BlockListIterator {
    cursor: Option<Block>,
}

impl Iterator for BlockListIterator {
    type Item = Block;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = self.cursor;
        if let Some(cur) = self.cursor {
            self.cursor = cur.load_next_block();
        }
        ret
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

/// Largest object size allowed with our mimalloc implementation, in bytes
pub(crate) const MI_LARGE_OBJ_SIZE_MAX: usize =
    crate::util::rust_util::min_of_usize(Block::BYTES, MAX_BIN_SIZE);
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
        let mut cursor = list.first;
        while let Some(block) = cursor {
            pages += Block::BYTES >> crate::util::constants::LOG_BYTES_IN_PAGE;
            cursor = block.load_next_block();
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
