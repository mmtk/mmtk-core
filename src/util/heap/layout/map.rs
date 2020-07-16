use crate::util::generic_freelist::GenericFreeList;
use crate::util::heap::freelistpageresource::CommonFreeListPageResource;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::Address;

pub trait Map: Sized {
    type FreeList: GenericFreeList;

    fn new() -> Self;

    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor);

    fn create_freelist(&self, pr: &CommonFreeListPageResource) -> Box<Self::FreeList>;

    fn create_parent_freelist(
        &self,
        pr: &CommonFreeListPageResource,
        units: usize,
        grain: i32,
    ) -> Box<Self::FreeList>;

    fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        head: Address,
    ) -> Address;

    fn get_next_contiguous_region(&self, start: Address) -> Address;

    fn get_contiguous_region_chunks(&self, start: Address) -> usize;

    fn get_contiguous_region_size(&self, start: Address) -> usize;

    fn free_all_chunks(&self, any_chunk: Address);

    fn free_contiguous_chunks(&self, start: Address) -> usize;

    fn boot(&self) {}

    fn finalize_static_space_map(&self, from: Address, to: Address);

    fn is_finalized(&self) -> bool;

    fn get_discontig_freelist_pr_ordinal(&self, pr: &CommonFreeListPageResource) -> usize;

    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor;

    fn get_chunk_index(&self, address: Address) -> usize {
        address >> LOG_BYTES_IN_CHUNK
    }

    fn address_for_chunk_index(&self, chunk: usize) -> Address {
        unsafe { Address::from_usize(chunk << LOG_BYTES_IN_CHUNK) }
    }

    fn add_to_cumulative_committed_pages(&self, pages: usize);

    fn get_cumulative_committed_pages(&self) -> usize;
}
