use super::sft::*;
use crate::util::Address;
#[cfg(debug_assertions)]
use crate::util::ObjectReference;
#[cfg(debug_assertions)]
use crate::vm::VMBinding;

pub trait SFTMap {
    /// Check if the address has an SFT entry for it (including an empty SFT entry). This is mostly a bound check
    /// to make sure that we won't have an index-out-of-bound error. For the sake of performance, the implementation
    /// of other methods in this trait (such as get(), update() and clear()) does not need to do this check implicitly.
    /// Instead, they assume the address has a valid entry in the SFT. If an address could be arbitary, they should call this
    /// method as a pre-check before they call any other methods in the trait.
    fn has_sft_entry(&self, addr: Address) -> bool;

    fn get(&self, address: Address) -> &dyn SFT;
    fn update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize);
    fn clear(&self, address: Address);
    /// Make sure we have valid SFT entries for the object reference.
    #[cfg(debug_assertions)]
    fn assert_valid_entries_for_object<VM: VMBinding>(&self, object: ObjectReference) {
        use crate::vm::ObjectModel;
        let object_sft = self.get(object.to_address());
        let object_start_sft = self.get(VM::VMObjectModel::object_start_ref(object));

        debug_assert!(
            object_sft.name() != EMPTY_SFT_NAME,
            "Object {} has empty SFT",
            object
        );
        debug_assert_eq!(
            object_sft.name(),
            object_start_sft.name(),
            "Object {} has incorrect SFT entries (object start = {}, object = {}).",
            object,
            object_start_sft.name(),
            object_sft.name()
        );
    }
}

// On 64bits and when chunk-based sft table is not forced, we use space map.
#[cfg(all(target_pointer_width = "64", not(feature = "chunk_based_sft_table")))]
pub type SFTMapType<'a> = space_map::SFTSpaceMap<'a>;
// On 32bits, or when chunk-based sft table is forced, we use chunk map.
#[cfg(any(target_pointer_width = "32", feature = "chunk_based_sft_table"))]
pub type SFTMapType<'a> = chunk_map::SFTChunkMap<'a>;

#[allow(dead_code)]
mod space_map {
    use super::*;
    use crate::util::heap::layout::vm_layout_constants::{
        HEAP_START, LOG_SPACE_EXTENT, MAX_SPACE_EXTENT,
    };

    pub struct SFTSpaceMap<'a> {
        sft: Vec<&'a (dyn SFT + Sync + 'static)>,
    }

    unsafe impl<'a> Sync for SFTSpaceMap<'a> {}

    impl<'a> SFTMap for SFTSpaceMap<'a> {
        fn has_sft_entry(&self, _addr: Address) -> bool {
            // Address::ZERO is mapped to index 0, and Address::MAX is mapped to index 31 (TABLE_SIZE-1)
            // So any address has an SFT entry.
            true
        }

        fn get(&self, address: Address) -> &'a dyn SFT {
            self.sft[Self::addr_to_index(address)]
        }

        fn update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
            let mut_self = unsafe { self.mut_self() };
            let index = Self::addr_to_index(start);
            if cfg!(debug_assertions) {
                // Make sure we only update from empty to a valid space, or overwrite the space
                let old = mut_self.sft[index];
                assert!(old.name() == EMPTY_SFT_NAME || old.name() == space.name());
                // Make sure the range is in the space
                let space_start = Self::index_to_space_start(index);
                assert!(start >= space_start);
                assert!(start + bytes <= space_start + MAX_SPACE_EXTENT);
            }
            mut_self.sft[index] = space;
        }

        fn clear(&self, addr: Address) {
            let mut_self = unsafe { self.mut_self() };
            let index = Self::addr_to_index(addr);
            mut_self.sft[index] = &EMPTY_SPACE_SFT;
        }
    }

    impl<'a> SFTSpaceMap<'a> {
        /// This mask extracts a few bits from address, and use it as index to the space map table.
        /// This constant is specially picked for the current heap range (HEAP_STRAT/HEAP_END), and the space size (MAX_SPACE_EXTENT).
        /// If any of these changes, the test `test_address_arithmetic()` may fail, and this constant will need to be updated.
        /// Currently our spaces are using address range 0x0000_0200_0000_0000 to 0x0000_2200_0000_0000 (with a maximum of 16 spaces).
        /// When masked with this constant, the index is 1 to 16. If we mask any arbitrary address with this mask, we will get 0 to 31 (32 entries).
        pub const ADDRESS_MASK: usize = 0x0000_3f00_0000_0000usize;
        /// The table size for the space map.
        pub const TABLE_SIZE: usize = Self::addr_to_index(Address::MAX) + 1;

        /// Create a new space map.
        #[allow(clippy::assertions_on_constants)] // We assert to make sure the constants
        pub fn new() -> Self {
            debug_assert!(
                Self::TABLE_SIZE >= crate::util::heap::layout::heap_parameters::MAX_SPACES
            );
            Self {
                sft: vec![&EMPTY_SPACE_SFT; Self::TABLE_SIZE],
            }
        }

        // This is a temporary solution to allow unsafe mut reference.
        // FIXME: We need a safe implementation.
        #[allow(clippy::cast_ref_to_mut)]
        #[allow(clippy::mut_from_ref)]
        unsafe fn mut_self(&self) -> &mut Self {
            &mut *(self as *const _ as *mut _)
        }

        #[inline(always)]
        const fn addr_to_index(addr: Address) -> usize {
            // println!("addr          {:64x}", addr.as_usize());
            // println!("-mask       & {:64x}", mask);
            // println!("-after mask = {:64x}", addr & mask);
            addr.and(Self::ADDRESS_MASK) >> LOG_SPACE_EXTENT
        }

        const fn index_to_space_start(i: usize) -> Address {
            let (start, _) = Self::index_to_space_range(i);
            start
        }

        const fn index_to_space_range(i: usize) -> (Address, Address) {
            if i == 0 {
                panic!("Invalid index: there is no space for index 0")
            } else {
                (
                    HEAP_START.add((i - 1) << LOG_SPACE_EXTENT),
                    HEAP_START.add(i << LOG_SPACE_EXTENT),
                )
            }
        }
    }

    #[cfg(tests)]
    mod tests {
        use super::*;
        use crate::util::heap::layout::heap_parameters::MAX_SPACES;
        use crate::util::heap::layout::vm_layout_constants::{
            HEAP_END, HEAP_START, LOG_SPACE_EXTENT, MAX_SPACE_EXTENT,
        };

        // If the test `test_address_arithmetic()` fails, it is possible due to change of our heap range, max space extent, or max number of spaces.
        // We need to update the code and the constants for the address arithemtic.
        #[test]
        fn test_address_arithmetic() {
            // Before 1st space
            assert_eq!(SFTSpaceMap::addr_to_index(Address::ZERO), 0);
            assert_eq!(SFTSpaceMap::addr_to_index(HEAP_START - 1), 0);

            let assert_for_index = |i: usize| {
                let (start, end) = SFTSpaceMap::index_to_space_range(i);
                debug!("Space: Index#{} = [{}, {})", i, start, end);
                assert_eq!(SFTSpaceMap::addr_to_index(start), i);
                assert_eq!(SFTSpaceMap::addr_to_index(end - 1), i);
            };

            // Index 1 to 16 (MAX_SPACES)
            for i in 1..=MAX_SPACES {
                assert_for_index(i);
            }

            // assert space end
            let (_, last_space_end) = SFTSpaceMap::index_to_space_range(MAX_SPACES);
            println!("Space end = {}", last_space_end);
            println!("Heap  end = {}", HEAP_END);
            assert_eq!(last_space_end, HEAP_END);

            // after last space
            assert_eq!(SFTSpaceMap::addr_to_index(last_space_end), 17);
            assert_eq!(SFTSpaceMap::addr_to_index(Address::MAX), 31);
        }
    }
}

#[allow(dead_code)]
mod chunk_map {
    use super::*;
    use crate::util::conversions;
    use crate::util::conversions::*;
    use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
    use crate::util::heap::layout::vm_layout_constants::MAX_CHUNKS;

    pub struct SFTChunkMap<'a> {
        sft: Vec<&'a (dyn SFT + Sync + 'static)>,
    }

    // TODO: MMTK<VM> holds a reference to SFTChunkMap. We should have a safe implementation rather than use raw pointers for dyn SFT.
    unsafe impl<'a> Sync for SFTChunkMap<'a> {}

    impl<'a> SFTMap for SFTChunkMap<'a> {
        fn has_sft_entry(&self, addr: Address) -> bool {
            addr.chunk_index() < MAX_CHUNKS
        }

        fn get(&self, address: Address) -> &'a dyn SFT {
            debug_assert!(address.chunk_index() < MAX_CHUNKS);
            let res = unsafe { *self.sft.get_unchecked(address.chunk_index()) };
            if DEBUG_SFT {
                trace!(
                    "Get SFT for {} #{} = {}",
                    address,
                    address.chunk_index(),
                    res.name()
                );
            }
            res
        }

        /// Update SFT map for the given address range.
        /// It should be used when we acquire new memory and use it as part of a space. For example, the cases include:
        /// 1. when a space grows, 2. when initializing a contiguous space, 3. when ensure_mapped() is called on a space.
        fn update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
            if DEBUG_SFT {
                self.log_update(space, start, bytes);
            }
            let first = start.chunk_index();
            let last = conversions::chunk_align_up(start + bytes).chunk_index();
            for chunk in first..last {
                self.set(chunk, space);
            }
            if DEBUG_SFT {
                self.trace_sft_map();
            }
        }

        // TODO: We should clear a SFT entry when a space releases a chunk.
        #[allow(dead_code)]
        fn clear(&self, chunk_start: Address) {
            if DEBUG_SFT {
                debug!(
                    "Clear SFT for chunk {} (was {})",
                    chunk_start,
                    self.get(chunk_start).name()
                );
            }
            assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));
            let chunk_idx = chunk_start.chunk_index();
            self.set(chunk_idx, &EMPTY_SPACE_SFT);
        }
    }

    impl<'a> SFTChunkMap<'a> {
        pub fn new() -> Self {
            SFTChunkMap {
                sft: vec![&EMPTY_SPACE_SFT; MAX_CHUNKS],
            }
        }
        // This is a temporary solution to allow unsafe mut reference. We do not want several occurrence
        // of the same unsafe code.
        // FIXME: We need a safe implementation.
        #[allow(clippy::cast_ref_to_mut)]
        #[allow(clippy::mut_from_ref)]
        unsafe fn mut_self(&self) -> &mut Self {
            &mut *(self as *const _ as *mut _)
        }

        fn log_update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
            debug!("Update SFT for Chunk {} as {}", start, space.name(),);
            let first = start.chunk_index();
            let start_chunk = chunk_index_to_address(first);
            debug!(
                "Update SFT for {} bytes of Chunk {} #{}",
                bytes, start_chunk, first
            );
        }

        fn trace_sft_map(&self) {
            trace!("{}", self.print_sft_map());
        }

        // This can be used during debugging to print SFT map.
        fn print_sft_map(&self) -> String {
            // print the entire SFT map
            let mut res = String::new();

            const SPACE_PER_LINE: usize = 10;
            for i in (0..self.sft.len()).step_by(SPACE_PER_LINE) {
                let max = if i + SPACE_PER_LINE > self.sft.len() {
                    self.sft.len()
                } else {
                    i + SPACE_PER_LINE
                };
                let chunks: Vec<usize> = (i..max).collect();
                let space_names: Vec<&str> = chunks.iter().map(|&x| self.sft[x].name()).collect();
                res.push_str(&format!(
                    "{}: {}",
                    chunk_index_to_address(i),
                    space_names.join(",")
                ));
                res.push('\n');
            }

            res
        }

        // Currently only used by 32 bits vm map
        #[allow(dead_code)]
        pub fn clear_by_index(&self, chunk_idx: usize) {
            if DEBUG_SFT {
                let chunk_start = chunk_index_to_address(chunk_idx);
                debug!(
                    "Clear SFT for chunk {} by index (was {})",
                    chunk_start,
                    self.get(chunk_start).name()
                );
            }
            self.set(chunk_idx, &EMPTY_SPACE_SFT)
        }

        fn set(&self, chunk: usize, sft: &(dyn SFT + Sync + 'static)) {
            /*
             * This is safe (only) because a) this is only called during the
             * allocation and deallocation of chunks, which happens under a global
             * lock, and b) it only transitions from empty to valid and valid to
             * empty, so if there were a race to view the contents, in the one case
             * it would either see the new (valid) space or an empty space (both of
             * which are reasonable), and in the other case it would either see the
             * old (valid) space or an empty space, both of which are valid.
             */
            let self_mut = unsafe { self.mut_self() };
            // It is okay to set empty to valid, or set valid to empty. It is wrong if we overwrite a valid value with another valid value.
            if cfg!(debug_assertions) {
                let old = self_mut.sft[chunk].name();
                let new = sft.name();
                // Allow overwriting the same SFT pointer. E.g., if we have set SFT map for a space, then ensure_mapped() is called on the same,
                // in which case, we still set SFT map again.
                debug_assert!(
                    old == EMPTY_SFT_NAME || new == EMPTY_SFT_NAME || old == new,
                    "attempt to overwrite a non-empty chunk {} in SFT map (from {} to {})",
                    chunk,
                    old,
                    new
                );
            }
            self_mut.sft[chunk] = sft;
        }

        // pub fn is_in_any_space(&self, object: ObjectReference) -> bool {
        //     if object.to_address().chunk_index() >= self.sft.len() {
        //         return false;
        //     }
        //     self.get(object.to_address()).is_in_space(object)
        // }

        // #[cfg(feature = "is_mmtk_object")]
        // pub fn is_mmtk_object(&self, addr: Address) -> bool {
        //     if addr.chunk_index() >= self.sft.len() {
        //         return false;
        //     }
        //     self.get(addr).is_mmtk_object(addr)
        // }
    }
}
