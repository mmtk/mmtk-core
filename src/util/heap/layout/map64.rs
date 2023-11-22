use super::map::VMMap;
use crate::util::constants::*;
use crate::util::conversions;
use crate::util::freelist::FreeList;
use crate::util::heap::freelistpageresource::CommonFreeListPageResource;
use crate::util::heap::layout::heap_parameters::*;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::memory::MmapStrategy;
use crate::util::raw_memory_freelist::RawMemoryFreeList;
use crate::util::Address;
use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

const NON_MAP_FRACTION: f64 = 1.0 - 8.0 / 4096.0;

pub struct Map64 {
    inner: UnsafeCell<Map64Inner>,
}

struct Map64Inner {
    fl_page_resources: Vec<Option<NonNull<CommonFreeListPageResource>>>,
    fl_map: Vec<Option<NonNull<RawMemoryFreeList>>>,
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

unsafe impl Send for Map64 {}
unsafe impl Sync for Map64 {}

impl Map64 {
    pub fn new() -> Self {
        let mut high_water = vec![Address::ZERO; MAX_SPACES];
        let mut base_address = vec![Address::ZERO; MAX_SPACES];

        for i in 0..MAX_SPACES {
            let base = unsafe { Address::from_usize(i << vm_layout().log_space_extent) };
            high_water[i] = base;
            base_address[i] = base;
        }

        let descriptor_map = vec![SpaceDescriptor::UNINITIALIZED; MAX_SPACES];

        Self {
            inner: UnsafeCell::new(Map64Inner {
                descriptor_map,
                high_water,
                base_address,
                fl_page_resources: vec![None; MAX_SPACES],
                fl_map: vec![None; MAX_SPACES],
                finalized: false,
                cumulative_committed_pages: AtomicUsize::new(0),
            }),
        }
    }
}

impl VMMap for Map64 {
    fn insert(&self, start: Address, extent: usize, descriptor: SpaceDescriptor) {
        debug_assert!(Self::is_space_start(start));
        debug_assert!(extent <= vm_layout().space_size_64());
        // Each space will call this on exclusive address ranges. It is fine to mutate the descriptor map,
        // as each space will update different indices.
        let self_mut = unsafe { self.mut_self() };
        let index = Self::space_index(start).unwrap();
        self_mut.descriptor_map[index] = descriptor;
    }

    fn create_freelist(&self, start: Address) -> Box<dyn FreeList> {
        let units = vm_layout().space_size_64() >> LOG_BYTES_IN_PAGE;
        self.create_parent_freelist(start, units, units as _)
    }

    fn create_parent_freelist(
        &self,
        start: Address,
        mut units: usize,
        grain: i32,
    ) -> Box<dyn FreeList> {
        // This is only called during creating a page resource/space/plan/mmtk instance, which is single threaded.
        let self_mut = unsafe { self.mut_self() };
        let index = Self::space_index(start).unwrap();

        units = (units as f64 * NON_MAP_FRACTION) as _;
        let list_extent =
            conversions::pages_to_bytes(RawMemoryFreeList::size_in_pages(units as _, 1) as _);

        let heads = 1;
        let pages_per_block = RawMemoryFreeList::default_block_size(units as _, heads);
        let mut list = Box::new(RawMemoryFreeList::new(
            start,
            start + list_extent,
            pages_per_block,
            units as _,
            grain,
            heads,
            MmapStrategy::Normal,
        ));

        /*self_mut.fl_map[index] =
            Some(unsafe { &*(&list as &RawMemoryFreeList as *const RawMemoryFreeList) });
        */
        self_mut.fl_map[index] = unsafe { Some(NonNull::new_unchecked(&mut *list)) };
        /* Adjust the base address and highwater to account for the allocated chunks for the map */
        let base = conversions::chunk_align_up(start + list_extent);

        self_mut.high_water[index] = base;
        self_mut.base_address[index] = base;
        list
    }

    unsafe fn bind_freelist(&self, pr: *const CommonFreeListPageResource) {
        let index = Self::space_index((*pr).get_start()).unwrap();
        let self_mut = self.mut_self();
        self_mut.fl_page_resources[index] = Some(NonNull::new_unchecked(pr as _));
    }

    /// # Safety
    ///
    /// Caller must ensure that only one thread is calling this method.
    unsafe fn allocate_contiguous_chunks(
        &self,
        descriptor: SpaceDescriptor,
        chunks: usize,
        _head: Address,
    ) -> Address {
        debug_assert!(Self::space_index(descriptor.get_start()).unwrap() == descriptor.get_index());
        // Each space will call this on exclusive address ranges. It is fine to mutate the descriptor map,
        // as each space will update different indices.
        let self_mut = self.mut_self();

        let index = descriptor.get_index();
        let rtn = self.inner().high_water[index];
        let extent = chunks << LOG_BYTES_IN_CHUNK;
        self_mut.high_water[index] = rtn + extent;

        /* Grow the free list to accommodate the new chunks */
        let free_list = self.inner().fl_map[Self::space_index(descriptor.get_start()).unwrap()];
        if let Some(mut free_list) = free_list {
            let free_list = free_list.as_mut();
            free_list.grow_freelist(conversions::bytes_to_pages_up(extent) as _);
            let base_page = conversions::bytes_to_pages_up(rtn - self.inner().base_address[index]);
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

    unsafe fn free_contiguous_chunks(&self, _start: Address) -> usize {
        unreachable!()
    }

    fn boot(&self) {
        // This is only called during boot process by a single thread.
        // It is fine to get a mutable reference.
        let self_mut: &mut Map64Inner = unsafe { self.mut_self() };
        for pr in 0..MAX_SPACES {
            if let Some(mut fl) = self_mut.fl_map[pr] {
                let fl_mut: &mut RawMemoryFreeList = unsafe { fl.as_mut() };
                fl_mut.grow_freelist(0);
            }
        }
    }

    fn finalize_static_space_map(&self, _from: Address, _to: Address) {
        // This is only called during boot process by a single thread.
        // It is fine to get a mutable reference.
        let self_mut: &mut Map64Inner = unsafe { self.mut_self() };
        for pr in 0..MAX_SPACES {
            if let Some(mut fl) = self_mut.fl_page_resources[pr] {
                let fl_mut = unsafe { fl.as_mut() };
                fl_mut.resize_freelist(conversions::chunk_align_up(unsafe {
                    self.inner().fl_map[pr].unwrap().as_ref().get_limit()
                }));
            }
        }
        self_mut.finalized = true;
    }

    fn is_finalized(&self) -> bool {
        self.inner().finalized
    }

    fn get_descriptor_for_address(&self, address: Address) -> SpaceDescriptor {
        let index = Self::space_index(address).unwrap();
        self.inner().descriptor_map[index]
    }

    fn add_to_cumulative_committed_pages(&self, pages: usize) {
        self.inner()
            .cumulative_committed_pages
            .fetch_add(pages, Ordering::Relaxed);
    }
}

impl Map64 {
    /// # Safety
    ///
    /// The caller needs to guarantee there is no race condition. Either only one single thread
    /// is using this method, or multiple threads are accessing mutally exclusive data (e.g. different indices in arrays).
    /// In other cases, use mut_self_with_sync().
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Map64Inner {
        &mut *self.inner.get()
    }

    fn inner(&self) -> &Map64Inner {
        unsafe { &*self.inner.get() }
    }

    fn space_index(addr: Address) -> Option<usize> {
        if addr > vm_layout().heap_end {
            return None;
        }
        Some(addr >> vm_layout().space_shift_64())
    }

    fn is_space_start(base: Address) -> bool {
        (base & !vm_layout().space_mask_64()) == 0
    }
}

impl Default for Map64 {
    fn default() -> Self {
        Self::new()
    }
}
