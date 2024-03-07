use super::map::VMMap;
use crate::mmtk::SFT_MAP;
use crate::util::conversions;
use crate::util::freelist::FreeList;
use crate::util::heap::freelistpageresource::CommonFreeListPageResource;
use crate::util::heap::layout::heap_parameters::*;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::int_array_freelist::IntArrayFreeList;
use crate::util::Address;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

pub struct Map32 {
    sync: Mutex<Map32Sync>,

    // The following counters are read by `gc_poll`, so they must be fast to read.
    total_available_discontiguous_chunks: AtomicUsize,
    shared_discontig_fl_count: AtomicUsize,

    // TODO: Is this the right place for this field?
    // This used to be a global variable. When we remove global states, this needs to be put somewhere.
    // Currently I am putting it here, as for where this variable is used, we already have
    // references to vm_map - so it is convenient to put it here.
    cumulative_committed_pages: AtomicUsize,
}

struct Map32Sync {
    prev_link: Vec<i32>,
    next_link: Vec<i32>,
    region_map: IntArrayFreeList,
    global_page_map: IntArrayFreeList,
    shared_fl_map: Vec<Option<NonNull<CommonFreeListPageResource>>>,
    finalized: bool,
    descriptor_map: Vec<SpaceDescriptor>,
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
                shared_fl_map: vec![None; MAX_SPACES],
                finalized: false,
                descriptor_map: vec![SpaceDescriptor::UNINITIALIZED; max_chunks],
            }),
            total_available_discontiguous_chunks: AtomicUsize::new(0),
            shared_discontig_fl_count: AtomicUsize::new(0),
            cumulative_committed_pages: AtomicUsize::new(0),
        }
    }
}

impl VMMap for Map32 {
    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        let mut sync = self.sync();
        self.insert_no_lock(&mut *sync, start, extent, descriptor)
    }

    fn create_freelist(&self, _start: Address) -> Box<dyn FreeList> {
        let sync = self.sync();
        let ordinal = self.get_discontig_freelist_pr_ordinal();
        Box::new(IntArrayFreeList::from_parent(
            &sync.global_page_map,
            ordinal as _,
        ))
    }

    fn create_parent_freelist(
        &self,
        _start: Address,
        units: usize,
        grain: i32,
    ) -> Box<dyn FreeList> {
        Box::new(IntArrayFreeList::new(units, grain, 1))
    }

    unsafe fn bind_freelist(&self, pr: *const CommonFreeListPageResource) {
        let ordinal: usize = (*pr)
            .free_list
            .downcast_ref::<IntArrayFreeList>()
            .unwrap()
            .get_ordinal() as usize;
        // TODO: Remove `bind_freelist` completely by letting `Plan` enumerate spaces and their
        // underlying `FreeListPageResource` instances using `HasSpace::for_each_space_mut`.
        let mut sync = self.sync();
        sync.shared_fl_map[ordinal] = Some(NonNull::new_unchecked(pr as *mut _));
    }

    unsafe fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        head: Address,
    ) -> Address {
        let mut sync = self.sync();
        let chunk = sync.region_map.alloc(chunks as _);
        debug_assert!(chunk != 0);
        if chunk == -1 {
            return Address::zero();
        }
        self.total_available_discontiguous_chunks.fetch_sub(chunks, Ordering::SeqCst);
        let rtn = conversions::chunk_index_to_address(chunk as _);
        // Note: We are already holding the mutex with `sync`.
        // Don't call `self.insert()` or it will deadlock.
        self.insert_no_lock(&mut *sync, rtn, chunks << LOG_BYTES_IN_CHUNK, descriptor);
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
        // TODO: Remove this `sync` by replacing the global linked list with a per-space data
        // structure that records the chunks returned from `VMMap`.
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
        // TODO: Remove this `sync` by recording the size of each "contiguous region" returned from
        // `VMMap` in some per-space data structures.
        let sync = self.sync();
        sync.region_map.size(chunk as i32) as _
    }

    fn get_contiguous_region_size(&self, start: Address) -> usize {
        self.get_contiguous_region_chunks(start) << LOG_BYTES_IN_CHUNK
    }

    fn get_available_discontiguous_chunks(&self) -> usize {
        self.total_available_discontiguous_chunks.load(Ordering::SeqCst)
    }

    fn get_chunk_consumer_count(&self) -> usize {
        self.shared_discontig_fl_count.load(Ordering::SeqCst)
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
                self.free_contiguous_chunks_no_lock(&mut *sync, x);
            }
            while sync.prev_link[chunk] != 0 {
                let x = sync.prev_link[chunk];
                self.free_contiguous_chunks_no_lock(&mut *sync, x);
            }
            self.free_contiguous_chunks_no_lock(&mut *sync, chunk as _);
        }
    }

    unsafe fn free_contiguous_chunks(&self, start: Address) -> usize {
        debug!("free_contiguous_chunks: {}", start);
        let mut sync = self.sync();
        debug_assert!(start == conversions::chunk_align_down(start));
        let chunk = start.chunk_index();
        self.free_contiguous_chunks_no_lock(&mut *sync, chunk as _)
    }

    fn finalize_static_space_map(&self, from: Address, to: Address) {
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
        // TODO: Clippy favors using iter().flatten() rather than iter() with if-let.
        // https://rust-lang.github.io/rust-clippy/master/index.html#manual_flatten
        // Yi: I am not doing this refactoring right now, as I am not familiar with flatten() and
        // there is no test to ensure the refactoring will be correct.
        #[allow(clippy::manual_flatten)]
        for fl in sync.shared_fl_map.iter().copied() {
            if let Some(mut fl) = fl {
                // TODO: Remove `Map32Sync::shared_fl_map` to remove this `unsafe`.
                // With `HasSpace::for_each_space_mut`, we can let the `Plan` enumerate `Space` and
                // their `FreeListPageResource` instances instead of using the global map.
                // See also: https://github.com/mmtk/mmtk-core/pull/925
                let fl_mut = unsafe { fl.as_mut() };
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
            self.total_available_discontiguous_chunks.fetch_add(1, Ordering::SeqCst);
            sync.region_map.free(chunk_index as _, false); // put this chunk on the free list
            sync.global_page_map.set_uncoalescable(first_page);
            let alloced_pages = sync.global_page_map.alloc(PAGES_IN_CHUNK as _); // populate the global page map
            debug_assert!(alloced_pages == first_page);
            first_page += PAGES_IN_CHUNK as i32;
        }
        sync.finalized = true;
    }

    fn is_finalized(&self) -> bool {
        // TODO: This function is never called, so the sync doesn't matter.
        // Consider removing this function.
        let sync = self.sync();
        sync.finalized
    }

    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor {
        let index = address.chunk_index();
        // TODO: FIXME: This is going to hurt the performance because this function is used by
        // `Space::address_in_space`.  We should replace `descriptor_map` with SFT.
        let sync = self.sync();
        sync.descriptor_map[index]
    }

    fn add_to_cumulative_committed_pages(&self, pages: usize) {
        self.cumulative_committed_pages
            .fetch_add(pages, Ordering::Relaxed);
    }
}

impl Map32 {
    fn sync(&self) -> MutexGuard<Map32Sync> {
        self.sync.lock().unwrap()
    }

    fn insert_no_lock(&self, sync: &mut Map32Sync, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        let mut e = 0;
        while e < extent {
            let index = (start + e).chunk_index();
            assert!(
                sync.descriptor_map[index].is_empty(),
                "Conflicting virtual address request"
            );
            debug!(
                "Set descriptor {:?} for Chunk {}",
                descriptor,
                conversions::chunk_index_to_address(index)
            );
            sync.descriptor_map[index] = descriptor;
            //   VM.barriers.objectArrayStoreNoGCBarrier(spaceMap, index, space);
            e += BYTES_IN_CHUNK;
        }
    }

    fn free_contiguous_chunks_no_lock(&self, sync: &mut Map32Sync, chunk: i32) -> usize {
        let chunks = sync.region_map.free(chunk, false);
        self.total_available_discontiguous_chunks.fetch_add(chunks as usize, Ordering::SeqCst);
        let next = sync.next_link[chunk as usize];
        let prev = sync.prev_link[chunk as usize];
        if next != 0 {
            sync.prev_link[next as usize] = prev
        };
        if prev != 0 {
            sync.next_link[prev as usize] = next
        };
        sync.prev_link[chunk as usize] = 0;
        sync.next_link[chunk as usize] = 0;
        for offset in 0..chunks {
            let index = (chunk + offset) as usize;
            let chunk_start = conversions::chunk_index_to_address(index);
            debug!("Clear descriptor for Chunk {}", chunk_start);
            sync.descriptor_map[index] = SpaceDescriptor::UNINITIALIZED;
            unsafe {
                SFT_MAP.clear(chunk_start);
            }
        }
        chunks as _
    }

    fn get_discontig_freelist_pr_ordinal(&self) -> usize {
        // This counter is only modified during creating a page resource/space/plan/mmtk instance,
        // which is single threaded.  We may take advantage of this if we want to refactor the code
        // to remove this atomic variable.
        let old = self.shared_discontig_fl_count.fetch_add(1, Ordering::SeqCst);
        old + 1
    }
}

impl Default for Map32 {
    fn default() -> Self {
        Self::new()
    }
}
