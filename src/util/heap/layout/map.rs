use crate::util::freelist::FreeList;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::Address;
use crate::util::raw_memory_freelist::RawMemoryFreeList;

pub trait VMMap: Sync {
    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor);

    /// Create a free-list for a discontiguous space. Must only be called at boot time.
    fn create_freelist(&self, start: Address) -> Box<dyn FreeList>;

    /// Create a free-list for a contiguous space. Must only be called at boot time.
    fn create_parent_freelist(&self, start: Address, units: usize, grain: i32)
        -> Box<dyn FreeList>;

    fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        head: Address,
        maybe_raw_memory_freelist: Option<&mut RawMemoryFreeList>,
    ) -> Address;

    fn get_next_contiguous_region(&self, start: Address) -> Address;

    fn get_contiguous_region_chunks(&self, start: Address) -> usize;

    fn get_contiguous_region_size(&self, start: Address) -> usize;

    /// Return the total number of chunks available (unassigned) within the range of virtual memory
    /// apportioned to discontiguous spaces.
    fn get_available_discontiguous_chunks(&self) -> usize;

    /// Return the total number of clients contending for chunks. This is useful when establishing
    /// conservative bounds on the number of remaining chunks.
    fn get_chunk_consumer_count(&self) -> usize;

    fn free_all_chunks(&self, any_chunk: Address);

    fn free_contiguous_chunks(&self, start: Address) -> usize;

    fn finalize_static_space_map(
        &self,
        from: Address,
        to: Address,
        update_starts: &mut dyn FnMut(Address),
    );

    fn is_finalized(&self) -> bool;

    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor;

    fn add_to_cumulative_committed_pages(&self, pages: usize);
}
