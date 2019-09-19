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
use std::sync::atomic::{AtomicUsize};
use super::*;



type PR = FreeListPageResource<RegionSpace>;

#[derive(Debug)]
pub struct RegionSpace {
    common: UnsafeCell<CommonSpace<PR>>,
    pub alloc_regions: (Option<RegionRef>, Option<RegionRef>, Option<RegionRef>),
    lock: Mutex<()>,
    // pub alloc_region: (Option<Region>, Mutex<()>, usize),
    nursery_regions: AtomicUsize,
    total_regions: AtomicUsize,
    regions: Vec<RegionRef>,
    pub heap_size: usize,
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

    fn is_live(&self, _object: ObjectReference) -> bool {
        unreachable!()
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
            alloc_regions: (None, None, None),
            lock: Mutex::new(()),
            regions: vec![],
            nursery_regions: AtomicUsize::new(0),
            total_regions: AtomicUsize::new(0),
            heap_size: 0,
            // regions: RwLock::new(HashSet::with_capacity(997)),
        }
    }

    pub fn is_live_next(&self, object: ObjectReference) -> bool {
        Region::of_object(object).next_mark_table().is_marked(object)
    }

    pub fn is_live_prev(&self, object: ObjectReference) -> bool {
        Region::of_object(object).prev_mark_table().is_marked(object)
    }

    pub fn initialize_header(&self, object: ObjectReference, size: usize, is_mutator: bool, collector_full_trace: bool, in_marking: bool) {
        let region = Region::of_object(object);
        if is_mutator && in_marking {
            region.live_size.fetch_add(size, Ordering::Relaxed);
        }
        if is_mutator || collector_full_trace {
            region.next_mark_table().mark(object, false);
            region.prev_mark_table().mark(object, false);
        } else {
            region.next_mark_table().mark(object, false);
            region.prev_mark_table().mark(object, false);
        }
    }

    #[inline(always)]
    fn get_alloc_region(&self, gen: Gen) -> Option<RegionRef> {
        match gen {
            Gen::Eden => self.alloc_regions.0,
            Gen::Survivor => self.alloc_regions.1,
            Gen::Old => self.alloc_regions.2,
        }
    } 

    #[inline]
    fn refill_fast_once(region: Option<RegionRef>, size: usize) -> Option<Address> {
        if let Some(alloc_region) = region {
            alloc_region.allocate_par(size)
        } else {
            None
        }
    }

    #[inline]
    pub fn refill(&mut self, tls: *mut c_void, size: usize, generation: Gen) -> Option<Address> {
        debug_assert!(size < BYTES_IN_REGION, "Size too large {}", size);
        if let Some(a) = Self::refill_fast_once(self.get_alloc_region(generation), size) {
            return Some(a);
        }
        // Slow path
        let result = self.refill_slow(tls, size, generation);
        if result.is_none() {
            VMCollection::block_for_gc(tls);
        }
        result
    }

    #[inline(never)]
    fn refill_slow(&mut self, tls: *mut c_void, size: usize, generation: Gen) -> Option<Address> {
        let _lock = self.lock.lock().unwrap();
        // Try again
        if let Some(a) = Self::refill_fast_once(self.get_alloc_region(generation), size) {
            return Some(a);
        }
        // Acquire new region
        match self.acquire_with_lock(tls, PAGES_IN_REGION) {
            Some(region) => {
                if generation != Gen::Old {
                    self.nursery_regions.fetch_add(1, Ordering::SeqCst);
                }
                self.total_regions.fetch_add(1, Ordering::SeqCst);
                let region = Region::new(region, generation);
                let result = region.allocate(size).unwrap();
                match generation {
                    Gen::Eden => self.alloc_regions.0 = Some(region),
                    Gen::Survivor => self.alloc_regions.1 = Some(region),
                    Gen::Old => self.alloc_regions.2 = Some(region),
                }
                // println!("Alloc region {:?} {:?}", generation, region);
                Some(result)
            }
            None => {
                None
            },
        }
    }

    #[inline(never)]
    #[allow(dead_code)]
    pub fn refill_simple(&mut self, tls: *mut c_void, size: usize, generation: Gen) -> Option<Address> {
        lazy_static! {
            static ref LOCK: ::std::sync::Mutex<()> = ::std::sync::Mutex::new(());
        }
        let lock = LOCK.lock().unwrap();
        // Fast
        {
            if let Some(region) = self.get_alloc_region(generation) {
                if let Some(r) = region.allocate(size) {
                    return Some(r);
                }
            }
        }
        // Slow
        match self.acquire_with_lock(tls, PAGES_IN_REGION) {
            Some(region) => {
                if generation != Gen::Old {
                    self.nursery_regions.fetch_add(1, Ordering::SeqCst);
                }
                self.total_regions.fetch_add(1, Ordering::SeqCst);
                let region = Region::new(region, generation);
                let result = region.allocate(size).unwrap();
                // self.alloc_region.0 = Some(region);
                match generation {
                    Gen::Eden => self.alloc_regions.0 = Some(region),
                    Gen::Survivor => self.alloc_regions.1 = Some(region),
                    Gen::Old => self.alloc_regions.2 = Some(region),
                }
                // println!("Alloc region {:?} {:?}", generation, region);
                Some(result)
            }
            None => {
                ::std::mem::drop(lock);
                VMCollection::block_for_gc(tls);
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

    pub fn acquire_new_region(&mut self, tls: *mut c_void, generation: Gen) -> Option<RegionRef> {
        let address = self.acquire(tls, PAGES_IN_REGION);

        if !address.is_zero() {
            debug_assert!(address != embedded_meta_data::get_metadata_base(address));
            if generation != Gen::Old {
                self.nursery_regions.fetch_add(1, Ordering::SeqCst);
            }
            self.total_regions.fetch_add(1, Ordering::SeqCst);
            let mut region = Region::new(address, generation);
            region.get_mut().committed = true;
            Some(region)
        } else {
            None
        }
    }

    pub fn shift_mark_tables(&self) {
        for region in self.regions() {
            region.get_mut().shift_mark_table();
        }
    }

    pub fn clear_next_mark_tables(&self) {
        for region in self.regions() {
            region.get_mut().clear_next_mark_table();
        }
    }

    pub fn prepare(&mut self) {
        // let regions = self.regions.read().unwrap();
        // println!("RegionSpace prepare");
        {
            // let mut alloc_region = self.alloc_region.write().unwrap();
            self.alloc_regions.0 = None;
            self.alloc_regions.1 = None;
            self.alloc_regions.2 = None;
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
            self.alloc_regions.0 = None;
            self.alloc_regions.1 = None;
            self.alloc_regions.2 = None;
        }
        // Cleanup regions
        // let me = unsafe { &mut *(self as *mut Self) };
        // for region in self.regions() {
        //     if region.relocate {
        //         me.release_region(region);
        //     }
        // }
        for region in self.regions() {
            if !region.relocate {
                region.remset().clear_cards_in_collection_set();
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
            self.release_region(region);
        }
    }

    pub fn nursery_regions(&self) -> usize {
        self.nursery_regions.load(Ordering::Relaxed)
    }
    
    pub fn nursery_ratio(&self) -> f32 {
        let nursery = self.nursery_regions.load(Ordering::Relaxed) as f32;
        // let total = self.total_regions.load(Ordering::SeqCst) as f32;
        // let total = AVAILABLE_REGIONS_IN_HEAP as f32;
        debug_assert!(self.heap_size != 0);
        let total = (self.heap_size >> LOG_BYTES_IN_REGION) as f32;
        debug_assert!(total != 0f32);
        // println!("Nursery Ratio {}/{} = {}", nursery, total, nursery / total);
        // println!("Total: {}", total);
        // println!("Ratio: {}", nursery / total);
        nursery / total
    }

    pub fn committed_ratio(&self) -> f32 {
        let committed = self.total_regions.load(Ordering::Relaxed) as f32;
        debug_assert!(self.heap_size != 0);
        let total = (self.heap_size >> LOG_BYTES_IN_REGION) as f32;
        debug_assert!(total != 0f32);
        committed / total
    }

    fn release_region(&mut self, region: &Region) {
        if region.generation != Gen::Old {
            self.nursery_regions.fetch_sub(1, Ordering::SeqCst);
        }
        self.total_regions.fetch_sub(1, Ordering::SeqCst);
        region.get_mut().release();
        self.pr.as_mut().unwrap().release_pages(region.start());
    }

    #[inline]
    fn test_and_mark(object: ObjectReference, region: &Region) -> bool {
        region.next_mark_table().mark(object, true)
    }

    #[inline]
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference) -> ObjectReference {
        let region = Region::of_object(object);
        debug_assert!(region.start() != ::util::alloc::embedded_meta_data::get_metadata_base(region.start()), "Invalid region {:?}, object {:?}", region.start(), object);
        if Self::test_and_mark(object, region) {
            region.live_size.fetch_add(VMObjectModel::get_size_when_copied(object), Ordering::Relaxed);
            trace.process_node(object);
        }
        object
    }

    #[inline]
    pub fn trace_evacuate_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, allocator: Allocator, tls: *mut c_void) -> ObjectReference {
        let region = Region::of_object(object);
        debug_assert!(region.start() != ::util::alloc::embedded_meta_data::get_metadata_base(region.start()), "Invalid region {:?}, object {:?}", region.start(), object);
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

    // #[inline(never)]
    pub fn trace_evacuate_object_in_cset<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, allocator: Allocator, tls: *mut c_void) -> (ObjectReference, usize) {
        let region = Region::of_object(object);
        debug_assert!(region.start() != ::util::alloc::embedded_meta_data::get_metadata_base(region.start()), "Invalid region {:?}, object {:?}", region.start(), object);
        debug_assert!(region.committed);
        debug_assert!(region.relocate);
        let prior_status_word = ForwardingWord::attempt_to_forward(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(prior_status_word) {
            (ForwardingWord::spin_and_get_forwarded_object(object, prior_status_word), 0)
        } else {
            let new_object = ForwardingWord::forward_object(object, allocator, tls);
            trace.process_node(new_object);
            (new_object, VMObjectModel::get_current_size(new_object))
        }
    }

    pub fn compute_collection_set_for_nursery_gc(&self, available_pages: usize, predictor: impl AccumulativePauseTimePredictor) {
        const MAX_LIVE_SIZE: usize = (BYTES_IN_REGION as f64 * 0.65) as usize;
        let mut regions: Vec<RegionRef> = self.regions().collect();
        regions.sort_unstable_by_key(|r| r.live_size.load(Ordering::Relaxed));
        let avail_regions = (available_pages >> embedded_meta_data::LOG_PAGES_IN_REGION) * REGIONS_IN_CHUNK;
        let mut available_size = avail_regions << LOG_BYTES_IN_REGION;
        // Select all young regions
        for region in &regions {
            if region.generation != Gen::Old {
                // let live_size = meta.live_size.load(Ordering::Relaxed);
                // if live_size <= MAX_LIVE_SIZE && live_size < available_size {
                    // if DEBUG {
                        // println!("Relocate {:?} {:?}", region.generation, region);
                    // }
                    region.get_mut().relocate = true;
                    // available_size -= live_size;
                // }
            }
        }
    }

    pub fn compute_collection_set_for_mixed_gc(&self, available_pages: usize, mut predictor: impl AccumulativePauseTimePredictor) {
        // println!("Mixed GC");
        // FIXME: Bad performance
        const MAX_LIVE_SIZE: usize = (BYTES_IN_REGION as f64 * 0.65) as usize;
        let mut regions: Vec<RegionRef> = self.regions().collect();
        regions.sort_unstable_by_key(|r| r.live_size.load(Ordering::Relaxed));
        let avail_regions = (available_pages >> embedded_meta_data::LOG_PAGES_IN_REGION) * REGIONS_IN_CHUNK;
        let mut available_size = avail_regions << LOG_BYTES_IN_REGION;
        let mut n = 0;
        // Select all young regions
        for region in &regions {
            if region.generation != Gen::Old {
                let live_size = region.live_size.load(Ordering::Relaxed);
                if live_size <= MAX_LIVE_SIZE && live_size < available_size {
                    if DEBUG {
                        println!("Relocate {:?}", region);
                    }
                    predictor.record(region);
                    region.get_mut().relocate = true;
                    available_size -= live_size;
                    n += 1;
                    // println!("{} nursery regions, pause time = {} ms", n, predictor.predict());
                }
            }
        }
        // Select some old regions
        for region in regions {
            if region.generation == Gen::Old {
                let live_size = region.live_size.load(Ordering::Relaxed);
                if live_size <= MAX_LIVE_SIZE && live_size < available_size {
                    predictor.record(region);
                    if predictor.within_budget() {
                        if DEBUG {
                            println!("Relocate {:?}", region);
                        }
                        region.get_mut().relocate = true;
                        available_size -= live_size;
                        n += 1;
                        // println!("{} old regions, pause time = {} ms", n, predictor.predict_f32());
                    } else {
                        break;
                    }
                }
            }
        }
        // println!("Mixed GC CS={}", n);
    }

    pub fn compute_collection_set_full_heap(&self, available_pages: usize) {
        // FIXME: Bad performance
        const MAX_LIVE_SIZE: usize = (BYTES_IN_REGION as f64 * 0.65) as usize;
        let mut regions: Vec<RegionRef> = self.regions().collect();
        regions.sort_unstable_by_key(|r| r.live_size.load(Ordering::Relaxed));
        let avail_regions = (available_pages >> embedded_meta_data::LOG_PAGES_IN_REGION) * REGIONS_IN_CHUNK;
        let mut available_size = avail_regions << LOG_BYTES_IN_REGION;

        for region in regions {
            let live_size = region.live_size.load(Ordering::Relaxed);
            if live_size <= MAX_LIVE_SIZE && live_size < available_size {
                if DEBUG {
                    println!("Relocate {:?}", region);
                }
                region.get_mut().relocate = true;
                available_size -= live_size;
            }
        }
    }

    #[inline(always)]
    pub fn is_cross_region_ref(_src: ObjectReference, slot: Address, obj: ObjectReference) -> bool {
        if obj.is_null() {
            return false;
        }
        let x = slot.as_usize();
        let y = VMObjectModel::ref_to_address(obj).as_usize();
        ((x ^ y) >> LOG_BYTES_IN_REGION) != 0
    }
    
    pub fn prepare_to_iterate_regions_par(&self) {
        let me = unsafe { &mut *(self as *const _ as usize as *mut Self) };
        me.regions = self.regions().filter(|r| r.relocate).collect();
    }
    
    pub fn iterate_tospace_remset_roots<T: TraceLocal>(&self, trace: &T, id: usize, num_workers: usize, nursery: bool, timer: &PauseTimePredictionTimer) {
        let start_time = ::std::time::SystemTime::now();
        let size = (self.regions.len() + num_workers - 1) / num_workers;
        let start = size * id;
        let limit = size * (id + 1);
        let regions = self.regions.len();
        let limit = if limit > regions { regions } else { limit };
        let mut cards = 0;
        
        for i in start..limit {
            let region = &self.regions[i];
            debug_assert!(region.relocate);
            cards += self.iterate_region_remset_roots(region, trace, nursery);
        }
        let time = start_time.elapsed().unwrap().as_millis() as usize;
        timer.report_remset_card_scanning_time(time, cards);
    }

    fn iterate_region_remset_roots<T: TraceLocal>(&self, region: &Region, trace: &T, nursery: bool) -> usize {
        let mut cards = 0;
        region.remset().iterate(|card| {
            cards += 1;
            card.linear_scan(|obj| {
                // if ::plan::g1::SLOW_ASSERTIONS {
                //     Self::validate_remset_root(obj);
                // }
                let trace: &mut T = unsafe { &mut *(trace as *const _ as usize as *mut T) };
                trace.process_node(obj);
            }, !nursery);
            region.remset().remove_card(card);
        });
        cards
    }

    #[cfg(not(feature = "g1"))]
    fn validate_remset_root(obj: ObjectReference) {}

    #[cfg(feature = "g1")]
    fn validate_remset_root(obj: ObjectReference) {
        struct C<F: Fn(Address)>(F);
        impl <F: Fn(Address)> ::plan::TransitiveClosure for C<F> {
            #[inline(always)]
            fn process_edge(&mut self, _src: ObjectReference, slot: Address) {
                (self.0)(slot)
            }
            fn process_node(&mut self, _object: ObjectReference) {
                unreachable!();
            }
        }
        let mut closure = C(|slot| {
            let field = unsafe { slot.load::<ObjectReference>() };
            if field.is_null() {
                return
            }
            assert!(PLAN.is_mapped_object(field),
                "{:?}.{:?} -> unmapped {:?}", obj, slot, field
            );
            if PLAN.region_space.in_space(field) {
                assert!(Region::of_object(field).committed,
                    "{:?}.{:?} -> g1 {:?} but {:?} is released", obj, slot, field, Region::of_object(field)
                );
            }
        });
        VMScanning::scan_object(&mut closure, obj, 0 as _);
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

    pub fn validate_remsets(&self) {
        for region in self.regions() {
            region.prev_mark_table().iterate(region.start(), region.cursor, |src| {
                scan_edge(src, |slot| {
                    let obj = unsafe { slot.load::<ObjectReference>() };
                    if !obj.is_null() && self.in_space(obj) && Region::of_object(obj) != region {
                        let other_region = Region::of_object(obj);
                        use super::*;
                        if !other_region.remset().contains_card(Card::of(src)) {
                            println!(
                                "Card {:?} for <{:?}.{:?}> ({:?} {}) is not remembered by region {:?} {} ({:?})",
                                Card::of(src).0, src, slot, region, if region.relocate { "reloc" } else { "-" },
                                other_region, if other_region.relocate { "reloc" } else { "-" }, obj
                            );
                            if Card::of(src).get_state() == CardState::Dirty {
                                panic!("Card {:?} is dirty", Card::of(src).0);
                            } else {
                                panic!("Card is clean");
                            }
                        }
                    }
                });
            });
        }
        println!("[RemSet Validation Finished]")
    }
}

fn scan_edge<F: Fn(Address)>(object: ObjectReference, f: F) {
    struct ObjectFieldsClosure<F: Fn(Address)>(F);
    impl <F: Fn(Address)> ::plan::TransitiveClosure for ObjectFieldsClosure<F> {
        #[inline(always)]
        fn process_edge(&mut self, _src: ObjectReference, slot: Address) {
            (self.0)(slot)
        }
        fn process_node(&mut self, _object: ObjectReference) {
            unreachable!();
        }
    }
    let mut closure = ObjectFieldsClosure(f);
    VMScanning::scan_object(&mut closure, object, 0 as _);
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
    type Item = RegionRef;
    
    fn next(&mut self) -> Option<RegionRef> {
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
        let region = unsafe { Region::unchecked(self.cursor) };
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

impl ::std::fmt::Debug for RegionIterator {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        write!(f, "_")
    }
}
