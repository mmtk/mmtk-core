use std::sync::Mutex;
use std::sync::atomic::Ordering;
use util::heap::PageResource;
use util::heap::FreeListPageResource;
use util::heap::VMRequest;
use policy::space::{Space, CommonSpace};
use util::{Address, ObjectReference};
use plan::TransitiveClosure;
use plan::TraceLocal;
use util::forwarding_word as ForwardingWord;
use vm::ObjectModel;
use vm::VMObjectModel;
use plan::Allocator;
use super::region::*;
use util::alloc::embedded_meta_data;
use std::cell::UnsafeCell;
use libc::c_void;
use util::conversions;
use util::constants;
use vm::*;
use util::heap::layout::Mmapper;
use super::DEBUG;
use plan::selected_plan::PLAN;
use plan::plan::Plan;
use util::heap::layout::heap_layout::VM_MAP;



type PR = FreeListPageResource<RegionSpace>;

#[derive(Debug)]
pub struct RegionSpace {
    common: UnsafeCell<CommonSpace<PR>>,
    pub alloc_region: (Option<Region>, Mutex<()>, usize),
}

impl Space for RegionSpace {
    type PR = PR;

    fn common(&self) -> &CommonSpace<Self::PR> {
        unsafe {&*self.common.get()}
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<Self::PR> {
        &mut *self.common.get()
    }

    fn init(&mut self) {
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };
        if self.vmrequest.is_discontiguous() {
            self.pr = Some(FreeListPageResource::new_discontiguous(METADATA_PAGES_PER_CHUNK));
        } else {
            self.pr = Some(FreeListPageResource::new_contiguous(me, self.start, self.extent, METADATA_PAGES_PER_CHUNK));
        }
        self.pr.as_mut().unwrap().bind_space(me);
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if ForwardingWord::is_forwarded_or_being_forwarded(object) {
            return true;
        }
        Region::of(object).prev_mark_table().is_marked(object)
    }

    fn is_movable(&self) -> bool {
        true
    }

    fn grow_space(&self, start: Address, bytes: usize, new_chunk: bool) {
        if new_chunk {
            let chunk = conversions::chunk_align(start + bytes, true);
            ::util::heap::layout::heap_layout::MMAPPER.ensure_mapped(chunk, METADATA_PAGES_PER_CHUNK);
            VMMemory::zero(chunk, METADATA_PAGES_PER_CHUNK << constants::LOG_BYTES_IN_PAGE);
        }
    }

    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("not supported")
    }
}

impl RegionSpace {
    pub fn new(name: &'static str, vmrequest: VMRequest) -> Self {
        RegionSpace {
            common: UnsafeCell::new(CommonSpace::new(name, true, false, true, vmrequest)),
            alloc_region: (None, Mutex::new(()), 0),
            // regions: RwLock::new(HashSet::with_capacity(997)),
        }
    }

    pub fn is_live_current(&self, object: ObjectReference) -> bool {
        Region::of(object).curr_mark_table().is_marked(object)
    }

    pub fn initialize_header(&self, object: ObjectReference) {
        Region::of(object).prev_mark_table().mark(object, false);
    }

    #[inline]
    fn refill_fast_once(region: Option<Region>, size: usize) -> Option<Address> {
        if let Some(alloc_region) = region {
            alloc_region.allocate_par(size)
        } else {
            None
        }
    }

    #[inline]
    pub fn refill(&mut self, tls: *mut c_void, size: usize) -> Option<Address> {
        debug_assert!(self.alloc_region.2 != tls as usize);
        debug_assert!(size < BYTES_IN_REGION, "Size too large {}", size);
        if let Some(a) = Self::refill_fast_once(self.alloc_region.0, size) {
            return Some(a);
        }
        // Slow path
        let result = self.refill_slow(tls, size);
        if result.is_none() {
            VMCollection::block_for_gc(tls);
        }
        result
    }

    #[inline(never)]
    fn refill_slow(&mut self, tls: *mut c_void, size: usize) -> Option<Address> {
        debug_assert!(self.alloc_region.2 != tls as usize);
        let _alloc_region = self.alloc_region.1.lock().unwrap();
        self.alloc_region.2 = tls as _;
        // Try again
        if let Some(a) = Self::refill_fast_once(self.alloc_region.0, size) {
            self.alloc_region.2 = 0;
            return Some(a);
        }
        // Acquire new region
        match self.acquire_with_lock(tls, PAGES_IN_REGION) {
            Some(region) => {
                let region = Region::new(region);
                let result = region.allocate(size).unwrap();
                self.alloc_region.0 = Some(region);
                self.alloc_region.2 = 0;
                Some(result)
            }
            None => {
                self.alloc_region.2 = 0;
                None
            },
        }
    }

    fn acquire_with_lock(&self, tls: *mut c_void, pages: usize) -> Option<Address> {
        let allow_poll = unsafe { VMActivePlan::is_mutator(tls) } && PLAN.is_initialized();

        let pr = self.common().pr.as_ref().unwrap();
        let pages_reserved = pr.reserve_pages(pages);

        // FIXME: Possibly unnecessary borrow-checker fighting
        let me = unsafe { &*(self as *const Self) };

        trace!("Polling ..");

        if allow_poll && VMActivePlan::global().poll::<PR>(false, me) {
            trace!("Collection required");
            pr.clear_request(pages_reserved);
            None
        } else {
            trace!("Collection not required");
            let rtn = pr.get_new_pages(pages_reserved, pages, self.common().zeroed, tls);
            if rtn.is_zero() {
                if !allow_poll {
                    panic!("Physical allocation failed when polling not allowed!");
                }
                let gc_performed = VMActivePlan::global().poll::<PR>(true, me);
                debug_assert!(gc_performed, "GC not performed when forced.");
                pr.clear_request(pages_reserved);
                None
            } else {
                Some(rtn)
            }
        }
    }

    pub fn swap_mark_tables(&self) {
        for mut region in self.regions() {
            region.swap_mark_tables();
        }
    }

    pub fn clear_next_mark_tables(&self) {
        for mut region in self.regions() {
            region.clear_next_mark_table();
        }
    }

    pub fn prepare(&mut self) {
        // let regions = self.regions.read().unwrap();
        // println!("RegionSpace prepare");
        {
            // let mut alloc_region = self.alloc_region.write().unwrap();
            self.alloc_region.0 = None;
        }
        for region in self.regions() {
            region.live_size.store(0, Ordering::Relaxed);
        }
        // println!("RegionSpace prepare done");
    }

    // pub fn assert_all_live_objects_are_forwarded(&mut self) {

    // }
    
    pub fn release(&mut self) {
        {
            // let mut alloc_region = self.alloc_region.write().unwrap();
            self.alloc_region.0 = None;
        }
        // Cleanup regions
        // let me = unsafe { &mut *(self as *mut Self) };
        // for region in self.regions() {
        //     if region.relocate {
        //         me.release_region(region);
        //     }
        // }
        for mut region in self.regions() {
            if !region.relocate {
                region.remset.clear_cards_in_collection_set();
            }
        }
        let mut to_be_released = {
            let mut to_be_released = vec![];
            for region in self.regions() {
                if region.relocate {
                    to_be_released.push(region);
                }
            }
            to_be_released
        };
        // regions.retain(|&r| !r.relocate);
        for region in &mut to_be_released {
            self.release_region(*region);
        }
    }

    fn release_region(&mut self, region: Region) {
        region.release();
        self.pr.as_mut().unwrap().release_pages(region.0);
    }

    #[inline]
    fn test_and_mark(object: ObjectReference, region: Region) -> bool {
        region.curr_mark_table().mark(object, true)
    }

    #[inline]
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference) -> ObjectReference {
        let region = Region::of(object);
        debug_assert!(region.0 != ::util::alloc::embedded_meta_data::get_metadata_base(region.0), "Invalid region {:?}, object {:?}", region.0, object);
        if Self::test_and_mark(object, region) {
            region.live_size.fetch_add(VMObjectModel::get_size_when_copied(object), Ordering::Relaxed);
            trace.process_node(object);
        }
        object
    }

    #[inline]
    pub fn trace_evacuate_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, allocator: Allocator, tls: *mut c_void) -> ObjectReference {
        let region = Region::of(object);
        debug_assert!(region.0 != ::util::alloc::embedded_meta_data::get_metadata_base(region.0), "Invalid region {:?}, object {:?}", region.0, object);
        if region.relocate {
            let prior_status_word = ForwardingWord::attempt_to_forward(object);
            if ForwardingWord::state_is_forwarded_or_being_forwarded(prior_status_word) {
                ForwardingWord::spin_and_get_forwarded_object(object, prior_status_word)
            } else {
                let new_object = ForwardingWord::forward_object(object, allocator, tls);
                trace.process_node(new_object);
                new_object
            }
        } else {
            if Self::test_and_mark(object, region) {
                trace.process_node(object);
            }
            object
        }
    }

    pub fn compute_collection_set(&self, available_pages: usize) {
        // FIXME: Bad performance
        const MAX_LIVE_SIZE: usize = (BYTES_IN_REGION as f64 * 0.65) as usize;
        let mut regions: Vec<Region> = self.regions().collect();
        regions.sort_unstable_by_key(|r| r.live_size.load(Ordering::Relaxed));
        let avail_regions = (available_pages >> embedded_meta_data::LOG_PAGES_IN_REGION) * REGIONS_IN_CHUNK;
        let mut available_size = avail_regions << LOG_BYTES_IN_REGION;

        for region in regions {
            let meta = region.metadata();
            let live_size = meta.live_size.load(Ordering::Relaxed);
            if live_size <= MAX_LIVE_SIZE && live_size < available_size {
                if DEBUG {
                    println!("Relocate {:?}", region);
                }
                meta.relocate = true;
                available_size -= live_size;
            }
        }
    }
    
    pub fn iterate_tospace_remset_roots<T: TraceLocal>(&self, trace: &T) {
        for region in self.regions() {
            if region.relocate {
                region.remset.iterate(|card| {
                    // println!("Scan card eva {:?}", card.0);
                    card.linear_scan(|obj| {
                        if PLAN.versatile_space.in_space(obj) && !PLAN.versatile_space.is_marked(obj) {
                            return
                        }
                        let trace: &mut T = unsafe { &mut *(trace as *const _ as usize as *mut T) };
                        trace.process_node(obj);
                    })
                })
            }
        }
    }

    #[inline]
    pub fn regions(&self) -> RegionIterator {
        debug_assert!(!self.contiguous);
        let start = self.head_discontiguous_region;
        let chunks = VM_MAP.get_contiguous_region_chunks(start);
        let limit = start + (chunks << embedded_meta_data::LOG_BYTES_IN_REGION);
        RegionIterator {
            space: unsafe { ::std::mem::transmute(self) },
            contingous_chunks: (start, limit),
            cursor: start,
        }
    }
}

impl ::std::ops::Deref for RegionSpace {
    type Target = CommonSpace<PR>;
    fn deref(&self) -> &CommonSpace<PR> {
        self.common()
    }
}

impl ::std::ops::DerefMut for RegionSpace {
    fn deref_mut(&mut self) -> &mut CommonSpace<PR> {
        self.common_mut()
    }
}

pub struct RegionIterator {
    space: &'static RegionSpace,
    contingous_chunks: (Address, Address), // (Start, Limit)
    cursor: Address,
}

impl RegionIterator {
    fn bump_cursor_to_next_region(&mut self) {
        let mut cursor = self.cursor;
        // Bump to next region
        cursor += BYTES_IN_REGION;
        // Acquire a new slice of contingous_chunks if cursor >= limit
        if cursor >= self.contingous_chunks.1 {
            let start = VM_MAP.get_next_contiguous_region(self.contingous_chunks.0);
            if start.is_zero() {
                cursor = unsafe { Address::zero() };
            } else {
                let chunks = VM_MAP.get_contiguous_region_chunks(start);
                let limit = start + (chunks << embedded_meta_data::LOG_BYTES_IN_REGION);
                self.contingous_chunks = (start, limit);
                cursor = start;
            }
        }
        self.cursor = cursor;
    }
}

impl Iterator for RegionIterator {
    type Item = Region;
    
    fn next(&mut self) -> Option<Region> {
        if self.cursor.is_zero() {
            return None;
        }
        // Continue searching if `cursor` points to a metadata region
        if self.cursor == self.contingous_chunks.0 {
            debug_assert!(VM_MAP.get_descriptor_for_address(self.cursor) == self.space.descriptor);
            self.bump_cursor_to_next_region();
            return self.next();
        }
        // Continue searching if `cursor` points to a free region
        let region = Region(self.cursor);
        if !region.committed {
            self.bump_cursor_to_next_region();
            return self.next();
        }
        // `cursor` points to a committed region
        debug_assert!(VM_MAP.get_descriptor_for_address(self.cursor) == self.space.descriptor);
        self.bump_cursor_to_next_region();
        Some(region)
    }
}

