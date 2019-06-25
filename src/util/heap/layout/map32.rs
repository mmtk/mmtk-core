use ::util::conversions;
use ::util::heap::layout::vm_layout_constants::*;
use ::util::constants::*;
use ::util::heap::layout::heap_parameters::*;
use ::util::Address;
use ::util::int_array_freelist::IntArrayFreeList;
use ::util::heap::PageResource;
use ::util::heap::FreeListPageResource;
use ::util::heap::freelistpageresource::CommonFreeListPageResource;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use ::policy::space::Space;
use ::util::generic_freelist::GenericFreeList;
use std::mem;

// use ::util::free::IntArrayFreeList;

const NON_MAP_FRACTION: f64 = 1.0 - 8.0 / 4096.0;
#[cfg(target_pointer_width = "32")]
const MAP_BASE_ADDRESS: Address = Address(0);

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
    descriptor_map: Vec<usize>,
}

impl Map32 {
    pub fn new() -> Self {
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
            descriptor_map: vec![0; MAX_CHUNKS],
        }
    }

    #[allow(mutable_transmutes)]
    pub fn insert(&self, start: Address, extent: usize, descriptor: usize) {
        let self_mut: &mut Self = unsafe { mem::transmute(self) };
        let mut e = 0;
        while e < extent {
            let index = self.get_chunk_index(start + e);
            assert!(self.descriptor_map[index] == 0, "Conflicting virtual address request");
            self_mut.descriptor_map[index] = descriptor;
            //   VM.barriers.objectArrayStoreNoGCBarrier(spaceMap, index, space);
            e += BYTES_IN_CHUNK;
        }
    }

    pub fn create_freelist(&self, pr: &CommonFreeListPageResource) -> IntArrayFreeList {
        IntArrayFreeList::from_parent(&self.global_page_map, self.get_discontig_freelist_pr_ordinal(pr) as _)
    }

    pub fn create_parent_freelist(&self, units: usize, grain: i32) -> IntArrayFreeList {
        IntArrayFreeList::new(units, grain, 1)
    }

    #[allow(mutable_transmutes)]
    pub fn allocate_contiguous_chunks(&self, descriptor: usize, chunks: usize, head: Address) -> Address {
        let self_mut: &mut Self = unsafe { mem::transmute(self) };
        let sync = self.sync.lock().unwrap();
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

    pub fn get_next_contiguous_region(&self, start: Address) -> Address {
        debug_assert!(start == conversions::chunk_align(start, true));
        let chunk = self.get_chunk_index(start);
        if chunk == 0 {
            unsafe { Address::zero() }
        } else if self.next_link[chunk] == 0 {
            unsafe { Address::zero() }
        } else {
            let a = self.next_link[chunk];
            self.address_for_chunk_index(a as _)
        }
    }

    pub fn get_contiguous_region_chunks(&self, start: Address) -> usize {
        debug_assert!(start == conversions::chunk_align(start, true));
        let chunk = self.get_chunk_index(start);
        self.region_map.size(chunk as i32) as _
    }
    
    pub fn get_contiguous_region_size(&self, start: Address) -> usize {
        self.get_contiguous_region_chunks(start) << LOG_BYTES_IN_CHUNK
    }

    pub fn free_all_chunks(&self, any_chunk: Address) {
        let sync = self.sync.lock().unwrap();
        debug_assert!(any_chunk == conversions::chunk_align(any_chunk, true));
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

    pub fn free_contiguous_chunks(&self, start: Address) -> usize {
        let sync = self.sync.lock().unwrap();
        debug_assert!(start == conversions::chunk_align(start, true));
        let chunk = self.get_chunk_index(start);
        self.free_contiguous_chunks_no_lock(chunk as _)
    }

    #[allow(mutable_transmutes)]
    fn free_contiguous_chunks_no_lock(&self, chunk: i32) -> usize {
        let self_mut: &mut Self = unsafe { mem::transmute(self) };
        let chunks = self_mut.region_map.free(chunk, false);
        self_mut.total_available_discontiguous_chunks += chunks as usize;
        let next = self.next_link[chunk as usize];
        let prev = self.prev_link[chunk as usize];
        if next != 0 { self_mut.prev_link[next as usize] = prev };
        if prev != 0 { self_mut.next_link[prev as usize] = next };
        self_mut.prev_link[chunk as usize] = 0;
        self_mut.next_link[chunk as usize] = 0;
        for offset in 0..chunks {
            self_mut.descriptor_map[(chunk + offset) as usize] = 0;
            // VM.barriers.objectArrayStoreNoGCBarrier(spaceMap, chunk + offset, null);
        }
        chunks as _
    }

    #[allow(mutable_transmutes)]
    pub fn finalize_static_space_map(&self) {
        let self_mut: &mut Self = unsafe { mem::transmute(self) };
        /* establish bounds of discontiguous space */
        let start_address = ::policy::space::get_discontig_start();
        let first_chunk = self.get_chunk_index(start_address);
        let last_chunk = self.get_chunk_index(::policy::space::get_discontig_end());
        let unavail_start_chunk = last_chunk + 1;
        let trailing_chunks = MAX_CHUNKS - unavail_start_chunk;
        let pages = (1 + last_chunk - first_chunk) * PAGES_IN_CHUNK;
        // start_address=0xb0000000, first_chunk=704, last_chunk=703, unavail_start_chunk=704, trailing_chunks=320, pages=0
        // startAddress=0x68000000 firstChunk=416 lastChunk=703 unavailStartChunk=704 trailingChunks=320 pages=294912
        self_mut.global_page_map.resize_parent_freelist(pages, pages as _);
        for fl in &mut self_mut.shared_fl_map {
            if let Some(fl) = fl {
                let fl_mut: &mut CommonFreeListPageResource = unsafe { mem::transmute(fl) };
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
        let mut alloced_chunk = self_mut.region_map.alloc(first_chunk as _);       // block out entire bottom of address range
        for chunk_index in first_chunk..(last_chunk + 1) {
            alloced_chunk = self_mut.region_map.alloc(1);
        }
        alloced_chunk = self_mut.region_map.alloc(trailing_chunks as _);
        debug_assert!(alloced_chunk == unavail_start_chunk as i32, "{} != {}", alloced_chunk, unavail_start_chunk);
        /* set up the global page map and place chunks on free list */
        let mut first_page = 0;
        for chunk_index in first_chunk..(last_chunk + 1) {
            self_mut.total_available_discontiguous_chunks += 1;
            self_mut.region_map.free(chunk_index as _, false);  // put this chunk on the free list
            self_mut.global_page_map.set_uncoalescable(first_page);
            let alloced_pages = self_mut.global_page_map.alloc(PAGES_IN_CHUNK as _); // populate the global page map
            debug_assert!(alloced_pages == first_page);
            first_page += PAGES_IN_CHUNK as i32;
        }
        self_mut.finalized = true;
    } 

    pub fn is_finalized(&self) -> bool {
        self.finalized
    }

    #[allow(mutable_transmutes)]
    pub fn get_discontig_freelist_pr_ordinal(&self, pr: &CommonFreeListPageResource) -> usize {
        let self_mut: &mut Self = unsafe { mem::transmute(self) };
        self_mut.shared_fl_map[self.shared_discontig_fl_count] = Some(unsafe { &*(pr as *const CommonFreeListPageResource) });
        self_mut.shared_discontig_fl_count += 1;
        self.shared_discontig_fl_count
    }

    pub fn get_descriptor_for_address(&self, address: Address) -> usize {
        let index = self.get_chunk_index(address);
        self.descriptor_map[index]
    }

    fn get_chunk_index(&self, address: Address) -> usize {
        address.0 >> LOG_BYTES_IN_CHUNK
    }

    fn address_for_chunk_index(&self, chunk: usize) -> Address {
        unsafe { Address::from_usize(chunk << LOG_BYTES_IN_CHUNK) }
    }

    pub fn get_available_discontiguous_chunks(&self) -> usize {
        return self.total_available_discontiguous_chunks;
    }

    pub fn get_chunk_consumer_count(&self) -> usize {
        return self.shared_discontig_fl_count;
    }
}