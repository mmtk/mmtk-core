use super::map::CreateFreeListResult;
use super::map::VMMap;
use crate::mmtk::SFT_MAP;
use crate::util::conversions;
use crate::util::freelist::FreeList;
use crate::util::heap::layout::heap_parameters::*;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::int_array_freelist::IntArrayFreeList;
use crate::util::raw_memory_freelist::RawMemoryFreeList;
use crate::util::Address;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

pub struct Map32 {
    sync: Mutex<Map32Sync>,
}

#[doc(hidden)]
struct Map32Sync {
    prev_link: Vec<i32>,
    next_link: Vec<i32>,
    region_map: IntArrayFreeList,
    global_page_map: IntArrayFreeList,
    shared_discontig_fl_count: usize,
    total_available_discontiguous_chunks: usize,
    finalized: bool,
    descriptor_map: Vec<SpaceDescriptor>,

    // TODO: Is this the right place for this field?
    // This used to be a global variable. When we remove global states, this needs to be put somewhere.
    // Currently I am putting it here, as for where this variable is used, we already have
    // references to vm_map - so it is convenient to put it here.
    cumulative_committed_pages: AtomicUsize,
}

unsafe impl Send for Map32 {}
unsafe impl Sync for Map32 {}

impl Map32 {
    pub fn new() -> Self {
        let max_chunks = vm_layout().max_chunks();
        Map32 {
            sync: Mutex::new(Map32Sync {
                prev_link: vec![0; max_chunks],
                next_link: vec![0; max_chunks],
                region_map: IntArrayFreeList::new(max_chunks, max_chunks as _, 1),
                global_page_map: IntArrayFreeList::new(1, 1, MAX_SPACES),
                shared_discontig_fl_count: 0,
                total_available_discontiguous_chunks: 0,
                finalized: false,
                descriptor_map: vec![SpaceDescriptor::UNINITIALIZED; max_chunks],
                cumulative_committed_pages: AtomicUsize::new(0),
            }),
        }
    }
}

impl VMMap for Map32 {
    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        let mut sync = self.sync();
        sync.insert_no_lock(start, extent, descriptor)
    }

    fn create_freelist(&self, _start: Address) -> CreateFreeListResult {
        let mut sync = self.sync();
        let ordinal = sync.get_discontig_freelist_pr_ordinal() as i32;
        let free_list = Box::new(IntArrayFreeList::from_parent(
            &mut sync.global_page_map,
            ordinal,
        ));
        CreateFreeListResult {
            free_list,
            space_displacement: 0,
        }
    }

    fn create_parent_freelist(
        &self,
        _start: Address,
        units: usize,
        grain: i32,
    ) -> CreateFreeListResult {
        let free_list = Box::new(IntArrayFreeList::new(units, grain, 1));
        CreateFreeListResult {
            free_list,
            space_displacement: 0,
        }
    }

    unsafe fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        head: Address,
        _maybe_raw_memory_freelist: Option<&mut RawMemoryFreeList>,
    ) -> Address {
        let mut sync = self.sync();
        let chunk = sync.region_map.alloc(chunks as _);
        debug_assert!(chunk != 0);
        if chunk == -1 {
            return Address::zero();
        }
        sync.total_available_discontiguous_chunks -= chunks;
        let rtn = conversions::chunk_index_to_address(chunk as _);
        sync.insert_no_lock(rtn, chunks << LOG_BYTES_IN_CHUNK, descriptor);
        if head.is_zero() {
            debug_assert!(sync.next_link[chunk as usize] == 0);
        } else {
            sync.next_link[chunk as usize] = head.chunk_index() as _;
            sync.prev_link[head.chunk_index()] = chunk;
        }
        debug_assert!(sync.prev_link[chunk as usize] == 0);
        rtn
    }

    fn get_next_contiguous_region(&self, start: Address) -> Address {
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = start.chunk_index();
        let sync = self.sync();
        if chunk == 0 || sync.next_link[chunk] == 0 {
            unsafe { Address::zero() }
        } else {
            let a = sync.next_link[chunk];
            conversions::chunk_index_to_address(a as _)
        }
    }

    fn get_contiguous_region_chunks(&self, start: Address) -> usize {
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = start.chunk_index();
        let sync = self.sync();
        sync.region_map.size(chunk as i32) as _
    }

    fn get_contiguous_region_size(&self, start: Address) -> usize {
        self.get_contiguous_region_chunks(start) << LOG_BYTES_IN_CHUNK
    }

    fn get_available_discontiguous_chunks(&self) -> usize {
        let sync = self.sync();
        sync.total_available_discontiguous_chunks
    }

    fn get_chunk_consumer_count(&self) -> usize {
        let sync = self.sync();
        sync.shared_discontig_fl_count
    }

    #[allow(clippy::while_immutable_condition)]
    fn free_all_chunks(&self, any_chunk: Address) {
        debug!("free_all_chunks: {}", any_chunk);
        let mut sync = self.sync();
        debug_assert!(any_chunk == conversions::chunk_align_down(any_chunk));
        if !any_chunk.is_zero() {
            let chunk = any_chunk.chunk_index();
            while sync.next_link[chunk] != 0 {
                let x = sync.next_link[chunk];
                sync.free_contiguous_chunks_no_lock(x);
            }
            while sync.prev_link[chunk] != 0 {
                let x = sync.prev_link[chunk];
                sync.free_contiguous_chunks_no_lock(x);
            }
            sync.free_contiguous_chunks_no_lock(chunk as _);
        }
    }

    unsafe fn free_contiguous_chunks(&self, start: Address) -> usize {
        debug!("free_contiguous_chunks: {}", start);
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = start.chunk_index();
        let mut sync = self.sync();
        sync.free_contiguous_chunks_no_lock(chunk as _)
    }

    fn finalize_static_space_map(
        &self,
        from: Address,
        to: Address,
        update_starts: &mut dyn FnMut(Address),
    ) {
        let mut sync = self.sync();
        /* establish bounds of discontiguous space */
        let start_address = from;
        let first_chunk = start_address.chunk_index();
        let last_chunk = to.chunk_index();
        let unavail_start_chunk = last_chunk + 1;
        let trailing_chunks = vm_layout().max_chunks() - unavail_start_chunk;
        let pages = (1 + last_chunk - first_chunk) * PAGES_IN_CHUNK;
        // start_address=0xb0000000, first_chunk=704, last_chunk=703, unavail_start_chunk=704, trailing_chunks=320, pages=0
        // startAddress=0x68000000 firstChunk=416 lastChunk=703 unavailStartChunk=704 trailingChunks=320 pages=294912
        sync.global_page_map.resize_freelist(pages, pages as _);

        update_starts(start_address);

        // [
        //  2: -1073741825
        //  3: -1073741825
        //  5: -2147482624
        //  2048: -2147483648
        //  2049: -2147482624
        //  2050: 1024
        //  2051: 1024
        // ]
        /* set up the region map free list */
        sync.region_map.alloc(first_chunk as _); // block out entire bottom of address range
        for _ in first_chunk..=last_chunk {
            sync.region_map.alloc(1);
        }
        let alloced_chunk = sync.region_map.alloc(trailing_chunks as _);
        debug_assert!(
            alloced_chunk == unavail_start_chunk as i32,
            "{} != {}",
            alloced_chunk,
            unavail_start_chunk
        );
        /* set up the global page map and place chunks on free list */
        let mut first_page = 0;
        for chunk_index in first_chunk..=last_chunk {
            sync.total_available_discontiguous_chunks += 1;
            sync.region_map.free(chunk_index as _, false); // put this chunk on the free list
            sync.global_page_map.set_uncoalescable(first_page);
            let alloced_pages = sync.global_page_map.alloc(PAGES_IN_CHUNK as _); // populate the global page map
            debug_assert!(alloced_pages == first_page);
            first_page += PAGES_IN_CHUNK as i32;
        }
        sync.finalized = true;
    }

    fn is_finalized(&self) -> bool {
        let sync = self.sync();
        sync.finalized
    }

    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor {
        let index = address.chunk_index();
        let sync = self.sync();
        sync.descriptor_map[index]
    }

    fn add_to_cumulative_committed_pages(&self, pages: usize) {
        let sync = self.sync();
        sync.cumulative_committed_pages
            .fetch_add(pages, Ordering::Relaxed);
    }
}

impl Map32 {
    fn sync(&self) -> MutexGuard<Map32Sync> {
        self.sync.lock().unwrap()
    }
}

impl Map32Sync {
    fn insert_no_lock(&mut self, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        let mut e = 0;
        while e < extent {
            let index = (start + e).chunk_index();
            assert!(
                self.descriptor_map[index].is_empty(),
                "Conflicting virtual address request"
            );
            debug!(
                "Set descriptor {:?} for Chunk {}",
                descriptor,
                conversions::chunk_index_to_address(index)
            );
            self.descriptor_map[index] = descriptor;
            //   VM.barriers.objectArrayStoreNoGCBarrier(spaceMap, index, space);
            e += BYTES_IN_CHUNK;
        }
    }
    fn free_contiguous_chunks_no_lock(&mut self, chunk: i32) -> usize {
        unsafe {
            let chunks = self.region_map.free(chunk, false);
            self.total_available_discontiguous_chunks += chunks as usize;
            let next = self.next_link[chunk as usize];
            let prev = self.prev_link[chunk as usize];
            if next != 0 {
                self.prev_link[next as usize] = prev
            };
            if prev != 0 {
                self.next_link[prev as usize] = next
            };
            self.prev_link[chunk as usize] = 0;
            self.next_link[chunk as usize] = 0;
            for offset in 0..chunks {
                let index = (chunk + offset) as usize;
                let chunk_start = conversions::chunk_index_to_address(index);
                debug!("Clear descriptor for Chunk {}", chunk_start);
                self.descriptor_map[index] = SpaceDescriptor::UNINITIALIZED;
                SFT_MAP.clear(chunk_start);
            }
            chunks as _
        }
    }

    fn get_discontig_freelist_pr_ordinal(&mut self) -> usize {
        self.shared_discontig_fl_count += 1;
        self.shared_discontig_fl_count
    }
}

impl Default for Map32 {
    fn default() -> Self {
        Self::new()
    }
}
