use super::map::Map;
use crate::util::constants::*;
use crate::util::conversions;
use crate::util::generic_freelist::GenericFreeList;
use crate::util::heap::freelistpageresource::CommonFreeListPageResource;
use crate::util::heap::layout::heap_parameters::*;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::raw_memory_freelist::RawMemoryFreeList;
use crate::util::zeroed_alloc::new_zeroed_vec;
use crate::util::Address;
use std::sync::atomic::{AtomicUsize, Ordering};

const NON_MAP_FRACTION: f64 = 1.0 - 8.0 / 4096.0;

pub struct Map64 {
    fl_page_resources: Vec<Option<&'static CommonFreeListPageResource>>,
    fl_map: Vec<Option<&'static RawMemoryFreeList>>,
    finalized: bool,
    descriptor_map: Vec<SpaceDescriptor>,
    base_address: Vec<Address>,
    high_water: Vec<Address>,

    // TODO: Is this the right place for this field?
    // This used to be a global variable. When we remove global states, this needs to be put somewhere.
    // Currently I am putting it here, as for where this variable is used, we already have
    // references to vm_map - so it is convenient to put it here.
    cumulative_committed_pages: AtomicUsize,
}

impl Map for Map64 {
    type FreeList = RawMemoryFreeList;

    fn new() -> Self {
        let mut high_water = vec![Address::ZERO; MAX_SPACES];
        let mut base_address = vec![Address::ZERO; MAX_SPACES];

        for i in 0..MAX_SPACES {
            let base = unsafe { Address::from_usize(i << LOG_SPACE_SIZE_64) };
            high_water[i] = base;
            base_address[i] = base;
        }

        Self {
            // Note: descriptor_map is very large. Although it is initialized to
            // SpaceDescriptor(0), the compiler and the standard library are not smart enough to
            // elide the storing of 0 for each of the element.  Using standard vector creation,
            // such as `vec![SpaceDescriptor::UNINITIALIZED; MAX_CHUNKS]`, will cause severe
            // slowdown during start-up.
            descriptor_map: unsafe { new_zeroed_vec::<SpaceDescriptor>(MAX_CHUNKS) },
            high_water,
            base_address,
            fl_page_resources: vec![None; MAX_SPACES],
            fl_map: vec![None; MAX_SPACES],
            finalized: false,
            cumulative_committed_pages: AtomicUsize::new(0),
        }
    }

    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        debug_assert!(Self::is_space_start(start));
        debug_assert!(extent <= SPACE_SIZE_64);
        // Each space will call this on exclusive address ranges. It is fine to mutate the descriptor map,
        // as each space will update different indices.
        let self_mut = unsafe { self.mut_self() };
        let index = Self::space_index(start).unwrap();
        self_mut.descriptor_map[index] = descriptor;
    }

    fn create_freelist(&self, start: Address) -> Box<Self::FreeList> {
        let units = SPACE_SIZE_64 >> LOG_BYTES_IN_PAGE;
        self.create_parent_freelist(start, units, units as _)
    }

    fn create_parent_freelist(
        &self,
        start: Address,
        mut units: usize,
        grain: i32,
    ) -> Box<Self::FreeList> {
        // This is only called during creating a page resource/space/plan/mmtk instance, which is single threaded.
        let self_mut = unsafe { self.mut_self() };
        let index = Self::space_index(start).unwrap();

        units = (units as f64 * NON_MAP_FRACTION) as _;
        let list_extent =
            conversions::pages_to_bytes(RawMemoryFreeList::size_in_pages(units as _, 1) as _);

        let heads = 1;
        let pages_per_block = RawMemoryFreeList::default_block_size(units as _, heads);
        let list = Box::new(RawMemoryFreeList::new(
            start,
            start + list_extent,
            pages_per_block,
            units as _,
            grain,
            heads,
        ));

        self_mut.fl_map[index] =
            Some(unsafe { &*(&list as &RawMemoryFreeList as *const RawMemoryFreeList) });

        /* Adjust the base address and highwater to account for the allocated chunks for the map */
        let base = conversions::chunk_align_up(start + list_extent);

        self_mut.high_water[index] = base;
        self_mut.base_address[index] = base;
        list
    }

    fn bind_freelist(&self, pr: &'static CommonFreeListPageResource) {
        let index = Self::space_index(pr.get_start()).unwrap();
        let self_mut = unsafe { self.mut_self() };
        self_mut.fl_page_resources[index] = Some(pr);
    }

    fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        _head: Address,
    ) -> Address {
        debug_assert!(Self::space_index(descriptor.get_start()).unwrap() == descriptor.get_index());
        // Each space will call this on exclusive address ranges. It is fine to mutate the descriptor map,
        // as each space will update different indices.
        let self_mut = unsafe { self.mut_self() };

        let index = descriptor.get_index();
        let rtn = self.high_water[index];
        let extent = chunks << LOG_BYTES_IN_CHUNK;
        self_mut.high_water[index] = rtn + extent;

        /* Grow the free list to accommodate the new chunks */
        let free_list = self.fl_map[Self::space_index(descriptor.get_start()).unwrap()];
        if let Some(free_list) = free_list {
            let free_list =
                unsafe { &mut *(free_list as *const _ as usize as *mut RawMemoryFreeList) };
            free_list.grow_freelist(conversions::bytes_to_pages(extent) as _);
            let base_page = conversions::bytes_to_pages(rtn - self.base_address[index]);
            for offset in (0..(chunks * PAGES_IN_CHUNK)).step_by(PAGES_IN_CHUNK) {
                free_list.set_uncoalescable((base_page + offset) as _);
                /* The 32-bit implementation requires that pages are returned allocated to the caller */
                free_list.alloc_from_unit(PAGES_IN_CHUNK as _, (base_page + offset) as _);
            }
        }
        rtn
    }

    fn get_next_contiguous_region(&self, _start: Address) -> Address {
        unreachable!()
    }

    fn get_contiguous_region_chunks(&self, _start: Address) -> usize {
        unreachable!()
    }

    fn get_contiguous_region_size(&self, _start: Address) -> usize {
        unreachable!()
    }

    fn get_available_discontiguous_chunks(&self) -> usize {
        panic!("We don't use discontiguous chunks for 64-bit!");
    }

    fn get_chunk_consumer_count(&self) -> usize {
        panic!("We don't use discontiguous chunks for 64-bit!");
    }

    fn free_all_chunks(&self, _any_chunk: Address) {
        unreachable!()
    }

    fn free_contiguous_chunks(&self, _start: Address) -> usize {
        unreachable!()
    }

    fn boot(&self) {
        // This is only called during boot process by a single thread.
        // It is fine to get a mutable reference.
        let self_mut: &mut Self = unsafe { self.mut_self() };
        for pr in 0..MAX_SPACES {
            if let Some(fl) = self_mut.fl_map[pr] {
                #[allow(clippy::cast_ref_to_mut)]
                let fl_mut: &mut RawMemoryFreeList = unsafe { &mut *(fl as *const _ as *mut _) };
                fl_mut.grow_freelist(0);
            }
        }
    }

    fn finalize_static_space_map(&self, _from: Address, _to: Address) {
        // This is only called during boot process by a single thread.
        // It is fine to get a mutable reference.
        let self_mut: &mut Self = unsafe { self.mut_self() };
        for pr in 0..MAX_SPACES {
            if let Some(fl) = self_mut.fl_page_resources[pr] {
                #[allow(clippy::cast_ref_to_mut)]
                let fl_mut: &mut CommonFreeListPageResource =
                    unsafe { &mut *(fl as *const _ as *mut _) };
                fl_mut.resize_freelist(conversions::chunk_align_up(
                    self.fl_map[pr].unwrap().get_limit(),
                ));
            }
        }
        self_mut.finalized = true;
    }

    fn is_finalized(&self) -> bool {
        self.finalized
    }

    #[inline]
    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor {
        let index = Self::space_index(address).unwrap();
        self.descriptor_map[index]
    }

    fn add_to_cumulative_committed_pages(&self, pages: usize) {
        self.cumulative_committed_pages
            .fetch_add(pages, Ordering::Relaxed);
    }
}

impl Map64 {
    /// # Safety
    ///
    /// The caller needs to guarantee there is no race condition. Either only one single thread
    /// is using this method, or multiple threads are accessing mutally exclusive data (e.g. different indices in arrays).
    /// In other cases, use mut_self_with_sync().
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    fn space_index(addr: Address) -> Option<usize> {
        if addr > HEAP_END {
            return None;
        }
        Some(addr >> SPACE_SHIFT_64)
    }

    fn is_space_start(base: Address) -> bool {
        (base & !SPACE_MASK_64) == 0
    }
}

impl Default for Map64 {
    fn default() -> Self {
        Self::new()
    }
}
