initSidebarItems({"constant":[["ACTIVE_CHUNK_METADATA_SPEC","Metadata spec for the active chunk byte"],["ACTIVE_PAGE_METADATA_SPEC","Metadata spec for the active page byte"],["OFFSET_MALLOC_METADATA_SPEC",""]],"fn":[["has_object_alloced_by_malloc","Check if there is an object allocated by malloc at the address."],["is_alloced_by_malloc","Check if a given object was allocated by malloc"],["is_chunk_mapped",""],["is_chunk_marked",""],["is_chunk_marked_unsafe",""],["is_marked",""],["is_marked_unsafe",""],["is_meta_space_mapped","Check if metadata is mapped for a range [addr, addr + size). Metadata is mapped per chunk, we will go through all the chunks for [address, address + size), and check if they are mapped. If any of the chunks is not mapped, return false. Otherwise return true."],["is_meta_space_mapped_for_address","Check if metadata is mapped for a given address. We check if the active chunk metadata is mapped, and if the active chunk bit is marked as well. If the chunk is mapped and marked, we consider the metadata for the chunk is properly mapped."],["is_offset_malloc",""],["is_page_marked",""],["is_page_marked_unsafe",""],["load128","Load u128 bits of side metadata"],["map_active_chunk_metadata","Eagerly map the active chunk metadata surrounding `chunk_start`"],["map_meta_space","We map the active chunk metadata (if not previously mapped), as well as the alloc bit metadata and active page metadata here. Note that if [addr, addr + size) crosses multiple chunks, we will map for each chunk."],["set_alloc_bit",""],["set_chunk_mark",""],["set_mark_bit",""],["set_offset_malloc_bit",""],["set_page_mark",""],["unset_alloc_bit",""],["unset_alloc_bit_unsafe",""],["unset_chunk_mark_unsafe",""],["unset_mark_bit",""],["unset_offset_malloc_bit_unsafe",""],["unset_page_mark_unsafe",""]],"struct":[["CHUNK_MAP_LOCK","Lock to synchronize the mapping of side metadata for a newly allocated chunk by malloc"],["CHUNK_METADATA",""],["MAX_METADATA_ADDRESS","Maximum metadata address for the ACTIVE_CHUNK_METADATA_SPEC which is used to check bounds"]]});