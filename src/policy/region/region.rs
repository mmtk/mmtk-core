use util::alloc::embedded_meta_data;
use util::constants;
use util::Address;
use util::ObjectReference;
use std::sync::atomic::{AtomicUsize, Ordering};
use vm::{VMObjectModel, ObjectModel, VMMemory, Memory};
use std::ops::{Deref, DerefMut};
use util::heap::layout::vm_layout_constants::*;
use super::{RemSet, MarkTable};

pub const LOG_PAGES_IN_REGION: usize = 8;
pub const PAGES_IN_REGION: usize = 1 << LOG_PAGES_IN_REGION; // 256
pub const LOG_BYTES_IN_REGION: usize = LOG_PAGES_IN_REGION + constants::LOG_BYTES_IN_PAGE as usize;
pub const BYTES_IN_REGION: usize = 1 << LOG_BYTES_IN_REGION;//BYTES_IN_PAGE * PAGES_IN_REGION; // 1048576
pub const REGION_MASK: usize = BYTES_IN_REGION - 1;// 0..011111111111

pub const REGIONS_IN_CHUNK: usize = embedded_meta_data::PAGES_IN_REGION / PAGES_IN_REGION - 1;
const METADATA_REGIONS_PER_CHUNK: usize = 1;
const METADATA_PAGES_PER_REGION: usize = METADATA_REGIONS_PER_CHUNK * PAGES_IN_REGION / REGIONS_IN_CHUNK;
const METADATA_BYTES_PER_REGION: usize = METADATA_PAGES_PER_REGION << constants::LOG_BYTES_IN_PAGE;
const METADATA_BYTES_PER_CHUNK: usize = METADATA_REGIONS_PER_CHUNK << LOG_BYTES_IN_REGION;
pub const METADATA_PAGES_PER_CHUNK: usize = METADATA_REGIONS_PER_CHUNK << LOG_PAGES_IN_REGION;

pub trait ToAddress {
    fn to_address(&self) -> Address;
}

impl ToAddress for Address {
    #[inline]
    fn to_address(&self) -> Address {
        *self
    }
}

impl ToAddress for ObjectReference {
    #[inline]
    fn to_address(&self) -> Address {
        VMObjectModel::object_start_ref(*self)
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Region(pub Address);

impl Region {
    pub fn new(addr: Address) -> Self {
        debug_assert!(addr != embedded_meta_data::get_metadata_base(addr));
        if super::DEBUG {
            println!("Alloc {:?} in chunk {:?}", addr, embedded_meta_data::get_metadata_base(addr));
        }
        let mut region = Region(addr);
        region.committed = true;
        region.cursor = region.0;
        region.remset = Box::leak(box RemSet::new());
        region.prev_mark_table = None;
        region.curr_mark_table = Some(MarkTable::new());
        region
    }

    pub fn release(self) {
        if super::DEBUG {
            println!("Release {:?} in chunk {:?}", self.0, embedded_meta_data::get_metadata_base(self.0));
        }
        self.metadata().release();
    }

    #[inline]
    pub fn align(address: Address) -> Address {
        unsafe { Address::from_usize(address.to_address().0 & !REGION_MASK) }
    }

    #[inline]
    pub fn of(a: Address) -> Region {
        Region(Self::align(a))
    }

    #[inline]
    pub fn of_object(o: ObjectReference) -> Region {
        Region(Self::align(VMObjectModel::ref_to_address(o)))
    }

    #[inline]
    pub fn index(&self) -> usize {
        let chunk = embedded_meta_data::get_metadata_base(self.0);
        let region_start = chunk + METADATA_BYTES_PER_CHUNK;
        debug_assert!(self.0 >= region_start, "Invalid region {:?}, chunk {:?}", self.0, chunk);
        let index = (self.0 - region_start) >> LOG_BYTES_IN_REGION;
        index
    }

    #[inline]
    pub fn heap_index(&self) -> usize {
        (self.0 - HEAP_START) >> LOG_BYTES_IN_REGION
    }

    #[inline]
    pub fn metadata(&self) -> &'static mut MetaData {
        debug_assert!(::std::mem::size_of::<MetaData>() <= METADATA_BYTES_PER_REGION);
        let chunk = embedded_meta_data::get_metadata_base(self.0);
        let index = self.index();
        let address = chunk + METADATA_BYTES_PER_REGION * index;
        unsafe {
            ::std::mem::transmute(address.0)
        }
    }
    
    #[inline]
    pub fn allocate(&self, tlab_size: usize) -> Option<Address> {
        let slot: &AtomicUsize = unsafe { ::std::mem::transmute(&self.cursor) };
        let old = slot.load(Ordering::SeqCst);
        let new = old + tlab_size;
        if new > self.0.as_usize() + BYTES_IN_REGION {
            return None
        } else {
            slot.store(new, Ordering::SeqCst);
            return Some(unsafe { Address::from_usize(old) });
        }
    }

    #[inline]
    pub fn allocate_par(&self, tlab_size: usize) -> Option<Address> {
        let slot: &AtomicUsize = unsafe { ::std::mem::transmute(&self.cursor) };
        // let mut spin = 0usize;
        loop {
            let old = slot.load(Ordering::SeqCst);
            let new = old + tlab_size;
            if new > self.0.as_usize() + BYTES_IN_REGION {
                return None
            }
            if old == slot.compare_and_swap(old, new, Ordering::SeqCst) {
                return Some(unsafe { Address::from_usize(old) })
            }
            // ::std::thread::yield_now();
        }
    }

    pub fn preceding_region(&self) -> Region {
        Region(self.0 - BYTES_IN_REGION)
    }
}

impl Deref for Region {
    type Target = MetaData;
    #[inline]
    fn deref(&self) -> &MetaData {
        self.metadata() as &'static MetaData
    }
}

impl DerefMut for Region {
    #[inline]
    fn deref_mut(&mut self) -> &mut MetaData {
        self.metadata()
    }
}


#[repr(C)]
#[derive(Debug)]
pub struct MetaData {
    pub committed: bool,
    pub live_size: AtomicUsize,
    pub relocate: bool,
    pub cursor: Address,
    pub remset: &'static mut RemSet,
    prev_mark_table: Option<MarkTable>,
    curr_mark_table: Option<MarkTable>,
}

impl MetaData {
    #[inline]
    pub fn get_region(&self) -> Region {
        let self_address: Address = unsafe { ::std::mem::transmute(self) };
        let chunk = embedded_meta_data::get_metadata_base(self_address);
        let index = (self_address - chunk) / METADATA_BYTES_PER_REGION;
        let region_start = chunk + (METADATA_REGIONS_PER_CHUNK << LOG_BYTES_IN_REGION);
        Region(region_start + (index << LOG_BYTES_IN_REGION))
    }

    #[inline]
    fn release(&mut self) {
        self.prev_mark_table = None;
        self.curr_mark_table = None;
        let _remset = unsafe { Box::from_raw(self.remset) };
        VMMemory::zero(Address::from_ptr(self as _), ::std::mem::size_of::<Self>());
    }

    pub fn prev_mark_table(&self) -> &MarkTable {
        if let Some(t) = self.prev_mark_table.as_ref() {
            t
        } else {
            self.curr_mark_table.as_ref().unwrap()
        }
    }

    pub fn curr_mark_table(&self) -> &MarkTable {
        self.curr_mark_table.as_ref().unwrap()
    }

    pub fn swap_mark_tables(&mut self) {
        ::std::mem::swap(&mut self.prev_mark_table, &mut self.curr_mark_table);
        // self.prev_mark_table = self.curr_mark_table;
        self.curr_mark_table = Some(MarkTable::new());
    }

    pub fn clear_next_mark_table(&mut self) {
        self.curr_mark_table = Some(MarkTable::new());
    }
}

pub const REGIONS_IN_HEAP: usize = (HEAP_END.as_usize() - HEAP_START.as_usize()) / BYTES_IN_REGION;
