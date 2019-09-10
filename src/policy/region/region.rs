use util::alloc::embedded_meta_data;
use util::constants;
use util::Address;
use util::ObjectReference;
use std::sync::atomic::{AtomicUsize, Ordering};
use vm::*;
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
pub struct Region(Address);

impl Region {
    pub fn new(addr: Address) -> Self {
        debug_assert!(addr != embedded_meta_data::get_metadata_base(addr));
        if super::DEBUG {
            println!("Alloc {:?} in chunk {:?}", addr, embedded_meta_data::get_metadata_base(addr));
        }
        let mut region = Region(addr);
        region.initialize(Region(addr));
        region
    }

    pub fn release(self) {
        if super::DEBUG {
            println!("Release {:?} in chunk {:?}", self.0, embedded_meta_data::get_metadata_base(self.0));
        }
        self.metadata().release();
    }

    #[inline]
    pub unsafe fn unchecked(a: Address) -> Self {
        debug_assert!(Self::align(a) == a);
        Self(a)
    }

    #[inline]
    pub fn start(&self) -> Address {
        self.0
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
        let address = chunk + ::std::mem::size_of::<MetaData>() * index;
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
pub struct MetaData {
    pub committed: bool,
    pub live_size: AtomicUsize,
    pub relocate: bool,
    pub cursor: Address,
    remset: Option<RemSet>,
    mark_table0: MarkTable,
    mark_table1: MarkTable,
    active_table: usize,
    inactivate_table_used: bool,
    pub card_offset_table: [Address; super::CARDS_IN_REGION],
}

impl MetaData {
    #[inline]
    pub fn remset(&self) -> &'static mut RemSet {
        let r: &RemSet = self.remset.as_ref().unwrap();
        unsafe { &mut *(r as *const _ as usize as *mut _) }
    }

    #[inline]
    pub fn get_region(&self) -> Region {
        let self_address: Address = unsafe { ::std::mem::transmute(self) };
        let chunk = embedded_meta_data::get_metadata_base(self_address);
        let index = (self_address - chunk) / METADATA_BYTES_PER_REGION;
        let region_start = chunk + (METADATA_REGIONS_PER_CHUNK << LOG_BYTES_IN_REGION);
        Region(region_start + (index << LOG_BYTES_IN_REGION))
    }
    
    #[inline]
    fn initialize(&mut self, region: Region) {
        self.committed = true;
        self.live_size.store(0, Ordering::SeqCst);
        self.cursor = region.0;
        self.relocate = false;
        self.remset = Some(RemSet::new());
        self.mark_table0.clear();
        self.mark_table1.clear();
        self.active_table = 0;
        self.inactivate_table_used = false;
    }

    #[inline]
    fn release(&mut self) {
        self.remset = None;
        self.committed = false;
        self.active_table = 0;
        self.inactivate_table_used = false;
        for i in 0..super::CARDS_IN_REGION {
            self.card_offset_table[i] = unsafe { Address::zero() };
        }
        // VMMemory::zero(Address::from_ptr(self as _), ::std::mem::size_of::<Self>());
    }

    pub fn next_mark_table(&self) -> &MarkTable {
        if self.active_table == 0 {
            &self.mark_table0
        } else {
            &self.mark_table1
        }
    }

    pub fn prev_mark_table(&self) -> &MarkTable {
        if self.active_table == 0 {
            &self.mark_table1
        } else {
            &self.mark_table0
        }
    }

    pub fn prev_mark_table_or_next(&self) -> &MarkTable {
        if !self.inactivate_table_used {
            return self.next_mark_table();
        }
        if self.active_table == 0 {
            &self.mark_table1
        } else {
            &self.mark_table0
        }
    }

    pub fn shift_mark_table(&mut self) {
        self.active_table = 1 - self.active_table;
        self.inactivate_table_used = true;
        if self.active_table == 0 {
            self.mark_table0.clear()
        } else {
            self.mark_table1.clear()
        }
    }

    pub fn clear_next_mark_table(&mut self) {
        if self.active_table == 0 {
            self.mark_table0.clear()
        } else {
            self.mark_table1.clear()
        }
    }
}

pub const REGIONS_IN_HEAP: usize = (HEAP_END.as_usize() - HEAP_START.as_usize()) / BYTES_IN_REGION;
