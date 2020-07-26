use super::map::Map;
use crate::mmtk::SFT_MAP;
use crate::util::conversions;
use crate::util::generic_freelist::GenericFreeList;
use crate::util::heap::freelistpageresource::CommonFreeListPageResource;
use crate::util::heap::layout::heap_parameters::*;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::int_array_freelist::IntArrayFreeList;
use crate::util::Address;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct Map32 {
    prev_link: Vec<i32>,
    next_link: Vec<i32>,
    region_map: IntArrayFreeList,
    global_page_map: IntArrayFreeList,
    shared_discontig_fl_count: usize,
    shared_fl_map: Vec<Option<&'static CommonFreeListPageResource>>,
    total_available_discontiguous_chunks: usize,
    finalized: bool,
    sync: Mutex<()>,
    descriptor_map: Vec<SpaceDescriptor>,

    // TODO: Is this the right place for this field?
    // This used to be a global variable. When we remove global states, this needs to be put somewhere.
    // Currently I am putting it here, as for where this variable is used, we already have
    // references to vm_map - so it is convenient to put it here.
    cumulative_committed_pages: AtomicUsize,
}

impl Map for Map32 {
    type FreeList = IntArrayFreeList;

    fn new() -> Self {
        Map32 {
            prev_link: vec![0; MAX_CHUNKS],
            next_link: vec![0; MAX_CHUNKS],
            region_map: IntArrayFreeList::new(MAX_CHUNKS, MAX_CHUNKS as _, 1),
            global_page_map: IntArrayFreeList::new(1, 1, MAX_SPACES),
            shared_discontig_fl_count: 0,
            shared_fl_map: vec![None; MAX_SPACES],
            total_available_discontiguous_chunks: 0,
            finalized: false,
            sync: Mutex::new(()),
            descriptor_map: vec![SpaceDescriptor::UNINITIALIZED; MAX_CHUNKS],
            cumulative_committed_pages: AtomicUsize::new(0),
        }
    }

    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        let self_mut: &mut Self = unsafe { self.mut_self() };
        let mut e = 0;
        while e < extent {
            let index = self.get_chunk_index(start + e);
            assert!(
                self.descriptor_map[index].is_empty(),
                "Conflicting virtual address request"
            );
            self_mut.descriptor_map[index] = descriptor;
            //   VM.barriers.objectArrayStoreNoGCBarrier(spaceMap, index, space);
            e += BYTES_IN_CHUNK;
        }
    }

    fn create_freelist(&self, pr: &CommonFreeListPageResource) -> Box<Self::FreeList> {
        box IntArrayFreeList::from_parent(
            &self.global_page_map,
            self.get_discontig_freelist_pr_ordinal(pr) as _,
        )
    }

    fn create_parent_freelist(
        &self,
        _pr: &CommonFreeListPageResource,
        units: usize,
        grain: i32,
    ) -> Box<Self::FreeList> {
        box IntArrayFreeList::new(units, grain, 1)
    }

    fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        head: Address,
    ) -> Address {
        let self_mut: &mut Self = unsafe { self.mut_self() };
        let _sync = self.sync.lock().unwrap();
        let chunk = self_mut.region_map.alloc(chunks as _);
        debug_assert!(chunk != 0);
        if chunk == -1 {
            return unsafe { Address::zero() };
        }
        self_mut.total_available_discontiguous_chunks -= chunks;
        let rtn = self.address_for_chunk_index(chunk as _);
        self.insert(rtn, chunks << LOG_BYTES_IN_CHUNK, descriptor);
        if head.is_zero() {
            debug_assert!(self.next_link[chunk as usize] == 0);
        } else {
            self_mut.next_link[chunk as usize] = self.get_chunk_index(head) as _;
            self_mut.prev_link[self.get_chunk_index(head)] = chunk;
        }
        debug_assert!(self.prev_link[chunk as usize] == 0);
        rtn
    }

    fn get_next_contiguous_region(&self, start: Address) -> Address {
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = self.get_chunk_index(start);
        if chunk == 0 || self.next_link[chunk] == 0 {
            unsafe { Address::zero() }
        } else {
            let a = self.next_link[chunk];
            self.address_for_chunk_index(a as _)
        }
    }

    fn get_contiguous_region_chunks(&self, start: Address) -> usize {
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = self.get_chunk_index(start);
        self.region_map.size(chunk as i32) as _
    }

    fn get_contiguous_region_size(&self, start: Address) -> usize {
        self.get_contiguous_region_chunks(start) << LOG_BYTES_IN_CHUNK
    }

    // The two while loops appear to not mutate the loop condition. However,
    // free_contiguous_chunks_no_lock() in the loop actually unsafely mutates the while loop condition.
    // The two while loops can end.
    // TODO: We need to check the correctness, especially check thread safety.
    #[allow(clippy::while_immutable_condition)]
    fn free_all_chunks(&self, any_chunk: Address) {
        let _sync = self.sync.lock().unwrap();
        debug_assert!(any_chunk == conversions::chunk_align_down(any_chunk));
        if !any_chunk.is_zero() {
            let chunk = self.get_chunk_index(any_chunk);
            while self.next_link[chunk] != 0 {
                let x = self.next_link[chunk];
                self.free_contiguous_chunks_no_lock(x);
            }
            while self.prev_link[chunk] != 0 {
                let x = self.prev_link[chunk];
                self.free_contiguous_chunks_no_lock(x);
            }
            self.free_contiguous_chunks_no_lock(chunk as _);
        }
    }

    fn free_contiguous_chunks(&self, start: Address) -> usize {
        let _sync = self.sync.lock().unwrap();
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = self.get_chunk_index(start);
        self.free_contiguous_chunks_no_lock(chunk as _)
    }

    fn finalize_static_space_map(&self, from: Address, to: Address) {
        let self_mut: &mut Self = unsafe { self.mut_self() };
        /* establish bounds of discontiguous space */
        let start_address = from;
        let first_chunk = self.get_chunk_index(start_address);
        let last_chunk = self.get_chunk_index(to);
        let unavail_start_chunk = last_chunk + 1;
        let trailing_chunks = MAX_CHUNKS - unavail_start_chunk;
        let pages = (1 + last_chunk - first_chunk) * PAGES_IN_CHUNK;
        // start_address=0xb0000000, first_chunk=704, last_chunk=703, unavail_start_chunk=704, trailing_chunks=320, pages=0
        // startAddress=0x68000000 firstChunk=416 lastChunk=703 unavailStartChunk=704 trailingChunks=320 pages=294912
        self_mut.global_page_map.resize_freelist(pages, pages as _);
        for fl in self_mut.shared_fl_map.iter() {
            if let Some(fl) = fl {
                #[allow(clippy::cast_ref_to_mut)]
                let fl_mut: &mut CommonFreeListPageResource =
                    unsafe { &mut *(fl as *const _ as *mut _) };
                fl_mut.resize_freelist(start_address);
            }
        }
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
        self_mut.region_map.alloc(first_chunk as _); // block out entire bottom of address range
        for _ in first_chunk..=last_chunk {
            self_mut.region_map.alloc(1);
        }
        let alloced_chunk = self_mut.region_map.alloc(trailing_chunks as _);
        debug_assert!(
            alloced_chunk == unavail_start_chunk as i32,
            "{} != {}",
            alloced_chunk,
            unavail_start_chunk
        );
        /* set up the global page map and place chunks on free list */
        let mut first_page = 0;
        for chunk_index in first_chunk..=last_chunk {
            self_mut.total_available_discontiguous_chunks += 1;
            self_mut.region_map.free(chunk_index as _, false); // put this chunk on the free list
            self_mut.global_page_map.set_uncoalescable(first_page);
            let alloced_pages = self_mut.global_page_map.alloc(PAGES_IN_CHUNK as _); // populate the global page map
            debug_assert!(alloced_pages == first_page);
            first_page += PAGES_IN_CHUNK as i32;
        }
        self_mut.finalized = true;
    }

    fn is_finalized(&self) -> bool {
        self.finalized
    }

    fn get_discontig_freelist_pr_ordinal(&self, pr: &CommonFreeListPageResource) -> usize {
        let self_mut: &mut Self = unsafe { self.mut_self() };
        self_mut.shared_fl_map[self.shared_discontig_fl_count] =
            Some(unsafe { &*(pr as *const CommonFreeListPageResource) });
        self_mut.shared_discontig_fl_count += 1;
        self.shared_discontig_fl_count
    }

    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor {
        let index = self.get_chunk_index(address);
        self.descriptor_map[index]
    }

    fn add_to_cumulative_committed_pages(&self, pages: usize) {
        self.cumulative_committed_pages
            .fetch_add(pages, Ordering::Relaxed);
    }

    fn get_cumulative_committed_pages(&self) -> usize {
        self.cumulative_committed_pages.load(Ordering::Relaxed)
    }
}

impl Map32 {
    // This is a temporary solution to allow unsafe mut reference. We do not want several occurrence
    // of the same unsafe code.
    // FIXME: We need a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    fn free_contiguous_chunks_no_lock(&self, chunk: i32) -> usize {
        let self_mut: &mut Self = unsafe { self.mut_self() };
        let chunks = self_mut.region_map.free(chunk, false);
        self_mut.total_available_discontiguous_chunks += chunks as usize;
        let next = self.next_link[chunk as usize];
        let prev = self.prev_link[chunk as usize];
        if next != 0 {
            self_mut.prev_link[next as usize] = prev
        };
        if prev != 0 {
            self_mut.next_link[prev as usize] = next
        };
        self_mut.prev_link[chunk as usize] = 0;
        self_mut.next_link[chunk as usize] = 0;
        for offset in 0..chunks {
            self_mut.descriptor_map[(chunk + offset) as usize] = SpaceDescriptor::UNINITIALIZED;
            SFT_MAP.clear((chunk + offset) as usize);
            // VM.barriers.objectArrayStoreNoGCBarrier(spaceMap, chunk + offset, null);
        }
        chunks as _
    }
}

impl Default for Map32 {
    fn default() -> Self {
        Self::new()
    }
}
