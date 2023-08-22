use super::sft::*;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;

/// SFTMap manages the SFT table, and mapping between addresses with indices in the table. The trait allows
/// us to have multiple implementations of the SFT table.
pub trait SFTMap {
    /// Check if the address has an SFT entry in the map (including an empty SFT entry). This is mostly a bound check
    /// to make sure that we won't have an index-out-of-bound error. For the sake of performance, the implementation
    /// of other methods in this trait (such as get_unchecked(), update() and clear()) does not need to do this check implicitly.
    /// Instead, they assume the address has a valid entry in the SFT. If an address could be arbitary, they should call this
    /// method as a pre-check before they call those methods in the trait. We also provide a method `get_checked()` which includes
    /// this check, and will return an empty SFT if the address is out of bound.
    fn has_sft_entry(&self, addr: Address) -> bool;

    /// Get the side metadata spec this SFT map uses.
    fn get_side_metadata(&self) -> Option<&SideMetadataSpec>;

    /// Get SFT for the address. The address must have a valid SFT entry in the table (e.g. from an object reference, or from an address
    /// that is known to be in our spaces). Otherwise, use `get_checked()`.
    ///
    /// # Safety
    /// The address must have a valid SFT entry in the map. Usually we know this if the address is from an object reference, or from our space address range.
    /// Otherwise, the caller should check with `has_sft_entry()` before calling this method, or use `get_checked()`.
    unsafe fn get_unchecked(&self, address: Address) -> &dyn SFT;

    /// Get SFT for the address. The address can be arbitrary. For out-of-bound access, an empty SFT will be returned.
    /// We only provide the checked version for `get()`, as it may be used to query arbitrary objects and addresses. Other methods like `update/clear/etc` are
    /// mostly used inside MMTk, and in most cases, we know that they are within our space address range.
    fn get_checked(&self, address: Address) -> &dyn SFT;

    /// Set SFT for the address range. The address must have a valid SFT entry in the table.
    ///
    /// # Safety
    /// The address must have a valid SFT entry in the map. Usually we know this if the address is from an object reference, or from our space address range.
    /// Otherwise, the caller should check with `has_sft_entry()` before calling this method.
    unsafe fn update(&self, space: *const (dyn SFT + Sync + 'static), start: Address, bytes: usize);

    /// Eagerly initialize the SFT table. For most implementations, it could be the same as update().
    /// However, we need this as a seprate method for SFTDenseChunkMap, as it needs to map side metadata first
    /// before setting the table.
    ///
    /// # Safety
    /// The address must have a valid SFT entry in the map. Usually we know this if the address is from an object reference, or from our space address range.
    /// Otherwise, the caller should check with `has_sft_entry()` before calling this method.
    unsafe fn eager_initialize(
        &self,
        space: *const (dyn SFT + Sync + 'static),
        start: Address,
        bytes: usize,
    ) {
        self.update(space, start, bytes);
    }

    /// Clear SFT for the address. The address must have a valid SFT entry in the table.
    ///
    /// # Safety
    /// The address must have a valid SFT entry in the map. Usually we know this if the address is from an object reference, or from our space address range.
    /// Otherwise, the caller should check with `has_sft_entry()` before calling this method.
    unsafe fn clear(&self, address: Address);
}

pub(crate) fn create_sft_map() -> Box<dyn SFTMap> {
    cfg_if::cfg_if! {
        if #[cfg(all(feature = "malloc_mark_sweep", target_pointer_width = "64"))] {
            // 64-bit malloc mark sweep needs a chunk-based SFT map, but the sparse map is not suitable for 64bits.
            Box::new(dense_chunk_map::SFTDenseChunkMap::new())
        } else if #[cfg(target_pointer_width = "64")] {
            use crate::util::heap::layout::vm_layout::vm_layout;
            if vm_layout().force_use_contiguous_spaces {
                Box::new(space_map::SFTSpaceMap::new())
            } else {
                Box::new(sparse_chunk_map::SFTSparseChunkMap::new())
            }
        } else if #[cfg(target_pointer_width = "32")] {
            Box::new(sparse_chunk_map::SFTSparseChunkMap::new())
        } else {
            compile_err!("Cannot figure out which SFT map to use.");
        }
    }
}

#[allow(dead_code)]
#[cfg(target_pointer_width = "64")] // This impl only works for 64 bits: 1. the mask is designed for our 64bit heap range, 2. on 64bits, all our spaces are contiguous.
mod space_map {
    use super::*;
    use crate::util::heap::layout::vm_layout::vm_layout;
    use std::cell::UnsafeCell;

    /// Space map is a small table, and it has one entry for each MMTk space.
    pub struct SFTSpaceMap {
        sft: UnsafeCell<Vec<*const (dyn SFT + Sync + 'static)>>,
    }

    unsafe impl Sync for SFTSpaceMap {}

    impl SFTMap for SFTSpaceMap {
        fn has_sft_entry(&self, _addr: Address) -> bool {
            // Address::ZERO is mapped to index 0, and Address::MAX is mapped to index 31 (TABLE_SIZE-1)
            // So any address has an SFT entry.
            true
        }

        fn get_side_metadata(&self) -> Option<&SideMetadataSpec> {
            None
        }

        fn get_checked(&self, address: Address) -> &dyn SFT {
            // We should be able to map the entire address range to indices in the table.
            debug_assert!(Self::addr_to_index(address) < unsafe { (*self.sft.get()).len() });
            unsafe { &**(*self.sft.get()).get_unchecked(Self::addr_to_index(address)) }
        }

        unsafe fn get_unchecked(&self, address: Address) -> &dyn SFT {
            &**(*self.sft.get()).get_unchecked(Self::addr_to_index(address))
        }

        unsafe fn update(
            &self,
            space: *const (dyn SFT + Sync + 'static),
            start: Address,
            bytes: usize,
        ) {
            let table_size = Self::addr_to_index(Address::MAX) + 1;
            let index = Self::addr_to_index(start);
            if cfg!(debug_assertions) {
                // Make sure we only update from empty to a valid space, or overwrite the space
                let old = (*self.sft.get())[index];
                assert!((*old).name() == EMPTY_SFT_NAME || (*old).name() == (*space).name());
                // Make sure the range is in the space
                let space_start = Self::index_to_space_start(index);
                // FIXME: Curerntly skip the check for the last space. The following works fine for MMTk internal spaces,
                // but the VM space is an exception. Any address after the last space is considered as the last space,
                // based on our indexing function. In that case, we cannot assume the end of the region is within the last space (with MAX_SPACE_EXTENT).
                if index != table_size - 1 {
                    assert!(start >= space_start);
                    assert!(start + bytes <= space_start + vm_layout().max_space_extent());
                }
            }

            *(*self.sft.get()).get_unchecked_mut(index) = std::mem::transmute(space);
        }

        unsafe fn clear(&self, addr: Address) {
            let index = Self::addr_to_index(addr);
            *(*self.sft.get()).get_unchecked_mut(index) = &EMPTY_SPACE_SFT;
        }
    }

    impl SFTSpaceMap {
        /// Create a new space map.
        #[allow(clippy::assertions_on_constants)] // We assert to make sure the constants
        pub fn new() -> Self {
            let table_size = Self::addr_to_index(Address::MAX) + 1;
            debug_assert!(table_size >= crate::util::heap::layout::heap_parameters::MAX_SPACES);
            Self {
                sft: UnsafeCell::new(vec![&EMPTY_SPACE_SFT; table_size]),
            }
        }

        fn addr_to_index(addr: Address) -> usize {
            addr.and(vm_layout().address_mask()) >> vm_layout().log_space_extent
        }

        fn index_to_space_start(i: usize) -> Address {
            let (start, _) = Self::index_to_space_range(i);
            start
        }

        fn index_to_space_range(i: usize) -> (Address, Address) {
            if i == 0 {
                panic!("Invalid index: there is no space for index 0")
            } else {
                let start = Address::ZERO.add(i << vm_layout().log_space_extent);
                let extent = 1 << vm_layout().log_space_extent;
                (start, start.add(extent))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::util::heap::layout::heap_parameters::MAX_SPACES;
        use crate::util::heap::layout::vm_layout::vm_layout;

        // If the test `test_address_arithmetic()` fails, it is possible due to change of our heap range, max space extent, or max number of spaces.
        // We need to update the code and the constants for the address arithemtic.
        #[test]
        fn test_address_arithmetic() {
            // Before 1st space
            assert_eq!(SFTSpaceMap::addr_to_index(Address::ZERO), 0);
            assert_eq!(SFTSpaceMap::addr_to_index(vm_layout().heap_start - 1), 0);

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
            println!("Heap  end = {}", vm_layout().heap_end);
            assert_eq!(last_space_end, vm_layout().heap_end);

            // after last space
            assert_eq!(SFTSpaceMap::addr_to_index(last_space_end), 17);
            assert_eq!(SFTSpaceMap::addr_to_index(Address::MAX), 31);
        }
    }
}

#[allow(dead_code)]
mod dense_chunk_map {
    use super::*;
    use crate::util::conversions;
    use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;
    use crate::util::metadata::side_metadata::spec_defs::SFT_DENSE_CHUNK_MAP_INDEX;
    use crate::util::metadata::side_metadata::*;
    use std::cell::UnsafeCell;
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

    /// SFTDenseChunkMap is a small table. It has one entry for each space in the table, and use
    /// side metadata to record the index for each chunk. This works for both 32 bits and 64 bits.
    /// However, its performance is expected to be suboptimal, compared to the sparse chunk map on
    /// 32 bits, and the space map on 64 bits. So usually we do not use this implementation. However,
    /// it provides some flexibility so we can set SFT at chunk basis for 64bits for decent performance.
    /// For example, when we use library malloc for mark sweep, we have no control of where the
    /// library malloc may allocate into, so we cannot use the space map. And using a sparse chunk map
    /// will be costly in terms of memory. In this case, the dense chunk map is a good solution.
    pub struct SFTDenseChunkMap {
        /// The dense table, one entry per space. We use side metadata to store the space index for each chunk.
        /// 0 is EMPTY_SPACE_SFT.
        sft: UnsafeCell<Vec<*const (dyn SFT + Sync + 'static)>>,
        /// A map from space name (assuming they are unique) to their index. We use this to know whether we have
        /// pushed &dyn SFT for a space, and to know its index.
        index_map: UnsafeCell<HashMap<String, usize>>,
    }

    unsafe impl Sync for SFTDenseChunkMap {}

    impl SFTMap for SFTDenseChunkMap {
        fn has_sft_entry(&self, addr: Address) -> bool {
            if SFT_DENSE_CHUNK_MAP_INDEX.is_mapped(addr) {
                let index = Self::addr_to_index(addr);
                index < self.sft().len() as u8
            } else {
                // We haven't mapped side metadata for the chunk, so we do not have an SFT entry for the address.
                false
            }
        }

        fn get_side_metadata(&self) -> Option<&SideMetadataSpec> {
            Some(&crate::util::metadata::side_metadata::spec_defs::SFT_DENSE_CHUNK_MAP_INDEX)
        }

        fn get_checked(&self, address: Address) -> &dyn SFT {
            if self.has_sft_entry(address) {
                unsafe {
                    &**self
                        .sft()
                        .get_unchecked(Self::addr_to_index(address) as usize)
                }
            } else {
                &EMPTY_SPACE_SFT
            }
        }

        unsafe fn get_unchecked(&self, address: Address) -> &dyn SFT {
            &**self
                .sft()
                .get_unchecked(Self::addr_to_index(address) as usize)
        }

        unsafe fn eager_initialize(
            &self,
            space: *const (dyn SFT + Sync + 'static),
            start: Address,
            bytes: usize,
        ) {
            let context = SideMetadataContext {
                global: vec![SFT_DENSE_CHUNK_MAP_INDEX],
                local: vec![],
            };
            if context.try_map_metadata_space(start, bytes).is_err() {
                panic!("failed to mmap metadata memory");
            }

            self.update(space, start, bytes);
        }

        unsafe fn update(
            &self,
            space: *const (dyn SFT + Sync + 'static),
            start: Address,
            bytes: usize,
        ) {
            // Check if we have an entry in self.sft for the space. If so, get the index.
            // If not, push the space pointer to the table and add an entry to the hahs map.
            let index: u8 = *(*self.index_map.get())
                .entry((*space).name().to_string())
                .or_insert_with(|| {
                    let count = self.sft().len();
                    (*self.sft.get()).push(space);
                    count
                }) as u8;

            // Iterate through the chunks and record the space index in the side metadata.
            let first_chunk = conversions::chunk_align_down(start);
            let last_chunk = conversions::chunk_align_up(start + bytes);
            let mut chunk = first_chunk;
            debug!(
                "update {} (chunk {}) to {} (chunk {})",
                start,
                first_chunk,
                start + bytes,
                last_chunk
            );
            while chunk < last_chunk {
                trace!("Update {} to index {}", chunk, index);
                SFT_DENSE_CHUNK_MAP_INDEX.store_atomic::<u8>(chunk, index, Ordering::SeqCst);
                chunk += BYTES_IN_CHUNK;
            }
            debug!("update done");
        }

        unsafe fn clear(&self, address: Address) {
            SFT_DENSE_CHUNK_MAP_INDEX.store_atomic::<u8>(
                address,
                Self::EMPTY_SFT_INDEX,
                Ordering::SeqCst,
            );
        }
    }

    impl SFTDenseChunkMap {
        /// Empty space is at index 0
        const EMPTY_SFT_INDEX: u8 = 0;

        pub fn new() -> Self {
            Self {
                /// Empty space is at index 0
                sft: UnsafeCell::new(vec![&EMPTY_SPACE_SFT]),
                index_map: UnsafeCell::new(HashMap::new()),
            }
        }

        pub fn addr_to_index(addr: Address) -> u8 {
            SFT_DENSE_CHUNK_MAP_INDEX.load_atomic::<u8>(addr, Ordering::Relaxed)
        }

        fn sft(&self) -> &Vec<*const (dyn SFT + Sync + 'static)> {
            unsafe { &*self.sft.get() }
        }

        fn index_map(&self) -> &HashMap<String, usize> {
            unsafe { &*self.index_map.get() }
        }
    }
}

#[allow(dead_code)]
mod sparse_chunk_map {
    use std::cell::UnsafeCell;

    use super::*;
    use crate::util::conversions;
    use crate::util::conversions::*;
    use crate::util::heap::layout::vm_layout::vm_layout;
    use crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK;

    /// The chunk map is a sparse table. It has one entry for each chunk in the address space we may use.
    pub struct SFTSparseChunkMap {
        sft: UnsafeCell<Vec<*const (dyn SFT + Sync + 'static)>>,
    }

    unsafe impl Sync for SFTSparseChunkMap {}

    impl SFTMap for SFTSparseChunkMap {
        fn has_sft_entry(&self, addr: Address) -> bool {
            addr.chunk_index() < vm_layout().max_chunks()
        }

        fn get_side_metadata(&self) -> Option<&SideMetadataSpec> {
            None
        }

        fn get_checked(&self, address: Address) -> &dyn SFT {
            if self.has_sft_entry(address) {
                unsafe { &**(*self.sft.get()).get_unchecked(address.chunk_index()) }
            } else {
                &EMPTY_SPACE_SFT
            }
        }

        unsafe fn get_unchecked(&self, address: Address) -> &dyn SFT {
            &**(*self.sft.get()).get_unchecked(address.chunk_index())
        }

        /// Update SFT map for the given address range.
        /// It should be used when we acquire new memory and use it as part of a space. For example, the cases include:
        /// 1. when a space grows, 2. when initializing a contiguous space, 3. when ensure_mapped() is called on a space.
        unsafe fn update(
            &self,
            space: *const (dyn SFT + Sync + 'static),
            start: Address,
            bytes: usize,
        ) {
            if DEBUG_SFT {
                self.log_update(&*space, start, bytes);
            }
            let first = start.chunk_index();
            let last = conversions::chunk_align_up(start + bytes).chunk_index();
            for chunk in first..last {
                self.set(chunk, &*space);
            }
            if DEBUG_SFT {
                self.trace_sft_map();
            }
        }

        // TODO: We should clear a SFT entry when a space releases a chunk.
        #[allow(dead_code)]
        unsafe fn clear(&self, chunk_start: Address) {
            if DEBUG_SFT {
                debug!(
                    "Clear SFT for chunk {} (was {})",
                    chunk_start,
                    self.get_checked(chunk_start).name()
                );
            }
            assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));
            let chunk_idx = chunk_start.chunk_index();
            self.set(chunk_idx, &EMPTY_SPACE_SFT);
        }
    }

    impl SFTSparseChunkMap {
        pub fn new() -> Self {
            SFTSparseChunkMap {
                sft: UnsafeCell::new(vec![&EMPTY_SPACE_SFT; vm_layout().max_chunks()]),
            }
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
            unsafe {
                for i in (0..(*self.sft.get()).len()).step_by(SPACE_PER_LINE) {
                    let max = if i + SPACE_PER_LINE > (*self.sft.get()).len() {
                        (*self.sft.get()).len()
                    } else {
                        i + SPACE_PER_LINE
                    };
                    let chunks: Vec<usize> = (i..max).collect();
                    let space_names: Vec<&str> = chunks
                        .iter()
                        .map(|&x| (*(*self.sft.get())[x]).name())
                        .collect();
                    res.push_str(&format!(
                        "{}: {}",
                        chunk_index_to_address(i),
                        space_names.join(",")
                    ));
                    res.push('\n');
                }
            }

            res
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

            // It is okay to set empty to valid, or set valid to empty. It is wrong if we overwrite a valid value with another valid value.
            if cfg!(debug_assertions) {
                let old = unsafe { (*(*self.sft.get())[chunk]).name() };
                let new = sft.name();
                // Allow overwriting the same SFT pointer. E.g., if we have set SFT map for a space, then ensure_mapped() is called on the same,
                // in which case, we still set SFT map again.
                debug_assert!(
                    old == EMPTY_SFT_NAME || new == EMPTY_SFT_NAME || old == new,
                    "attempt to overwrite a non-empty chunk {} ({}) in SFT map (from {} to {})",
                    chunk,
                    crate::util::conversions::chunk_index_to_address(chunk),
                    old,
                    new
                );
            }
            unsafe { *(*self.sft.get()).get_unchecked_mut(chunk) = sft };
        }
    }
}
