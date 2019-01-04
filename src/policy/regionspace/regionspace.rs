use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::Ordering;

use ::util::heap::PageResource;
use ::util::heap::FreeListPageResource;
use ::util::heap::VMRequest;
use ::util::constants::CARD_META_PAGES_PER_REGION;

use ::policy::space::{Space, CommonSpace};
use ::util::{Address, ObjectReference};
use ::plan::TransitiveClosure;
use ::util::forwarding_word as ForwardingWord;
use ::vm::ObjectModel;
use ::vm::VMObjectModel;
use ::plan::Allocator;
use super::region::*;
use util::alloc::embedded_meta_data;
use std::cell::UnsafeCell;
use libc::{c_void, mprotect, PROT_NONE, PROT_EXEC, PROT_WRITE, PROT_READ};
use std::collections::HashSet;
use util::conversions;
use util::constants;
use vm::{Memory, VMMemory};
use util::heap::layout::Mmapper;

type PR = FreeListPageResource<RegionSpace>;

#[derive(Debug)]
pub struct RegionSpace {
    common: UnsafeCell<CommonSpace<PR>>,
    pub regions: RwLock<HashSet<Region>>
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
        Region::of(object).metadata().mark_table.is_marked(object)
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
}

impl RegionSpace {
    pub fn new(name: &'static str, vmrequest: VMRequest) -> Self {
        RegionSpace {
            common: UnsafeCell::new(CommonSpace::new(name, true, false, true, vmrequest)),
            regions: RwLock::new(HashSet::with_capacity(997)),
        }
    }

    pub fn acquire_new_region(&self, tls: *mut c_void) -> Option<Region> {
        // Allocate
        let region = self.acquire(tls, PAGES_IN_REGION);
        debug_assert!(region != embedded_meta_data::get_metadata_base(region));

        if !region.is_zero() {
            if cfg!(debug) {
                println!("Region alloc {:?} in chunk {:?}", region, embedded_meta_data::get_metadata_base(region));
            }
            let mut region = Region(region);
            region.committed = true;
            Some(region)
        } else {
            None
        }
    }

    pub fn prepare(&mut self) {
        let regions = self.regions.read().unwrap();
        for region in regions.iter() {
            region.clone().mark_table.clear();
            region.live_size.store(0, Ordering::Relaxed);
        }
    }

    pub fn release(&mut self) {
        // Cleanup regions
        let me = unsafe { &mut *(self as *mut Self) };
        let mut regions = self.regions.write().unwrap();
        for region in regions.iter() {
            if region.relocate {
                region.clone().clear();
                me.pr.as_mut().unwrap().release_pages(region.0)
            }
        }
        regions.retain(|&r| !r.relocate);
    }

    #[inline]
    fn test_and_mark(object: ObjectReference) -> bool {
        Region::of(object).mark_table.test_and_mark(object)
    }

    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference) -> ObjectReference {
        if Self::test_and_mark(object) {
            Region::of(object).live_size.fetch_add(VMObjectModel::get_size_when_copied(object), Ordering::Relaxed);
            trace.process_node(object);
        }
        object
    }

    pub fn trace_evacuate_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference, allocator: Allocator, tls: *mut c_void) -> ObjectReference {
        if Region::of(object).relocate {
            let prior_status_word = ForwardingWord::attempt_to_forward(object);
            if ForwardingWord::state_is_forwarded_or_being_forwarded(prior_status_word) {
                ForwardingWord::spin_and_get_forwarded_object(object, prior_status_word)
            } else {
                let new_object = ForwardingWord::forward_object(object, allocator, tls);
                trace.process_node(new_object);
                new_object
            }
        } else {
            if Self::test_and_mark(object) {
                trace.process_node(object);
            }
            object
        }
    }

    pub fn compute_collection_set(&self) -> Vec<Region> {
        let mut regions: Vec<Region> = { self.regions.read().unwrap().iter().map(|r| *r).collect() };
        regions.sort_unstable_by_key(|r| r.live_size.load(Ordering::Relaxed));
        let min_live_size = (BYTES_IN_REGION as f64 * 0.65) as usize;
        regions.drain_filter(|r| r.live_size.load(Ordering::Relaxed) > min_live_size);
        for region in &mut regions {
            region.relocate = true;
        }
        debug_assert!(regions.iter().all(|&r| r.live_size.load(Ordering::Relaxed) > min_live_size));
        regions
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






