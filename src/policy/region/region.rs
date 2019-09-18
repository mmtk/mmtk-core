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

const LOG_LOGICAL_REGIONS_IN_CHUNK: usize = embedded_meta_data::LOG_PAGES_IN_REGION - LOG_PAGES_IN_REGION; // 2
const LOG_METADATA_PAGES_PER_REGION: usize = LOG_PAGES_IN_REGION - LOG_LOGICAL_REGIONS_IN_CHUNK;
const LOG_METADATA_BYTES_PER_REGION: usize = LOG_METADATA_PAGES_PER_REGION + (constants::LOG_BYTES_IN_PAGE as usize);
// const METADATA_BYTES_PER_REGION: usize = METADATA_PAGES_PER_REGION << constants::LOG_BYTES_IN_PAGE;
const METADATA_BYTES_PER_CHUNK: usize = METADATA_REGIONS_PER_CHUNK << LOG_BYTES_IN_REGION;
pub const METADATA_PAGES_PER_CHUNK: usize = METADATA_REGIONS_PER_CHUNK << LOG_PAGES_IN_REGION;

pub const REGIONS_IN_HEAP: usize = (HEAP_END.as_usize() - HEAP_START.as_usize()) / BYTES_IN_REGION;
// pub const AVAILABLE_REGIONS_IN_HEAP: usize = (AVAILABLE_END.as_usize() - AVAILABLE_START.as_usize()) / BYTES_IN_REGION;

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
pub struct Region {
    // start: Address,
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
    pub generation: Gen,
}

pub type RegionRef = &'static Region;


impl Region {
    // Constructors & Destructors
    pub fn new(addr: Address, generation: Gen) -> RegionRef {
        debug_assert!(addr != embedded_meta_data::get_metadata_base(addr));
        debug_assert!(::std::mem::size_of::<Self>() <= (1 << LOG_METADATA_BYTES_PER_REGION), "Region metadata size = {:?}", ::std::mem::size_of::<Self>());
        if super::DEBUG {
            println!("Alloc {:?} in chunk {:?}", addr, embedded_meta_data::get_metadata_base(addr));
        }
        let region = unsafe { Self::unchecked(addr) }.get_mut();
        // region.start = addr;
        region.committed = true;
        region.live_size.store(0, Ordering::SeqCst);
        region.cursor = addr;
        region.relocate = false;
        region.remset = Some(RemSet::new());
        region.mark_table0.clear();
        region.mark_table1.clear();
        region.active_table = 0;
        region.inactivate_table_used = false;
        region.generation = generation;
        region
    }

    pub fn release(&mut self) {
        self.remset = None;
        self.committed = false;
        self.active_table = 0;
        self.inactivate_table_used = false;
        for i in 0..super::CARDS_IN_REGION {
            self.card_offset_table[i] = unsafe { Address::zero() };
        }
    }

    // Static methods
    #[inline]
    pub unsafe fn unchecked(region: Address) -> RegionRef {
        debug_assert!(Self::align(region) == region);
        let chunk = embedded_meta_data::get_metadata_base(region);
        let index = {
            // let region_start = chunk + METADATA_BYTES_PER_CHUNK;
            // debug_assert!(region >= region_start, "Invalid region {:?}, chunk {:?}", region, chunk);
            // let index = (region - region_start) >> LOG_BYTES_IN_REGION;
            // let index = (region - chunk) >> LOG_BYTES_IN_REGION;
            let index = (region.as_usize() & (embedded_meta_data::BYTES_IN_REGION - 1)) >> LOG_BYTES_IN_REGION;
            debug_assert!(0 < index && index <= 3, " region={:?} chunk={:?} index={}", region, chunk, index);
            index
        };
        // let address = chunk + ::std::mem::size_of::<Self>() * index;
        let address = chunk.as_usize() | (index << LOG_METADATA_BYTES_PER_REGION);
        debug_assert!(chunk.as_usize() | (index << LOG_METADATA_BYTES_PER_REGION) == chunk.as_usize() + (index << LOG_METADATA_BYTES_PER_REGION),
            "{} {} {} {}",
            chunk.as_usize() & (index << LOG_METADATA_BYTES_PER_REGION), chunk.as_usize() + (index << LOG_METADATA_BYTES_PER_REGION),
            chunk.as_usize(), (index << LOG_METADATA_BYTES_PER_REGION),
        );
        debug_assert!((address + ::std::mem::size_of::<Self>()) <= (chunk.as_usize() + BYTES_IN_REGION));
        ::std::mem::transmute(address)
    }

    #[inline]
    pub fn align(address: Address) -> Address {
        unsafe { Address::from_usize(address.to_address().0 & !REGION_MASK) }
    }

    #[inline]
    pub fn of(a: Address) -> RegionRef {
        unsafe { Self::unchecked(Self::align(a)) }
    }

    #[inline]
    pub fn of_object(o: ObjectReference) -> RegionRef {
        unsafe { Self::unchecked(Self::align(VMObjectModel::ref_to_address(o))) }
    }

    #[inline]
    pub fn heap_index(region: Address) -> usize {
        debug_assert!(Self::align(region) == region);
        (region - HEAP_START) >> LOG_BYTES_IN_REGION
    }

    // Instance methods

    #[inline]
    pub fn get_mut(&self) -> &'static mut Region {
        unsafe { &mut *(self as *const _ as usize as *mut _) }
    }

    #[inline(always)]
    pub fn start(&self) -> Address {
        // self.start
        let addr = self as *const _ as usize;
        let offset = addr & embedded_meta_data::REGION_MASK;
        let index = offset >> LOG_METADATA_BYTES_PER_REGION;
        debug_assert!(index >= 1 && index <= 3);
        let base = addr & !embedded_meta_data::REGION_MASK;
        unsafe { Address::from_usize(base | (index << LOG_BYTES_IN_REGION)) }
    }


    // #[inline]
    // fn index(&self) -> usize {
    //     let chunk = embedded_meta_data::get_metadata_base(self.0);
    //     let region_start = chunk + METADATA_BYTES_PER_CHUNK;
    //     debug_assert!(self.0 >= region_start, "Invalid region {:?}, chunk {:?}", self.0, chunk);
    //     let index = (self.0 - region_start) >> LOG_BYTES_IN_REGION;
    //     index
    // }

    // #[inline]
    // pub fn heap_index(&self) -> usize {
    //     (self.start - HEAP_START) >> LOG_BYTES_IN_REGION
    // }

    // #[inline]
    // pub fn metadata(&self) -> &'static mut MetaData {
    //     debug_assert!(::std::mem::size_of::<MetaData>() <= METADATA_BYTES_PER_REGION);
    //     let chunk = embedded_meta_data::get_metadata_base(self.0);
    //     let index = self.index();
    //     let address = chunk + ::std::mem::size_of::<MetaData>() * index;
    //     unsafe {
    //         ::std::mem::transmute(address.0)
    //     }
    // }
    
    #[inline]
    pub fn allocate(&self, tlab_size: usize) -> Option<Address> {
        let slot: &AtomicUsize = unsafe { ::std::mem::transmute(&self.cursor) };
        let old = slot.load(Ordering::SeqCst);
        let new = old + tlab_size;
        if new > self.start().as_usize() + BYTES_IN_REGION {
            return None
        } else {
            slot.store(new, Ordering::SeqCst);
            return Some(unsafe { Address::from_usize(old) });
        }
    }

    #[inline]
    pub fn allocate_par(&self, tlab_size: usize) -> Option<Address> {
        let slot: &AtomicUsize = unsafe { ::std::mem::transmute(&self.cursor) };
        loop {
            let old = slot.load(Ordering::SeqCst);
            let new = old + tlab_size;
            if new > self.start().as_usize() + BYTES_IN_REGION {
                return None
            }
            if old == slot.compare_and_swap(old, new, Ordering::SeqCst) {
                return Some(unsafe { Address::from_usize(old) })
            }
        }
    }

    #[inline]
    pub fn remset(&self) -> &'static mut RemSet {
        let r: &RemSet = self.remset.as_ref().unwrap();
        unsafe { &mut *(r as *const _ as usize as *mut _) }
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

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
#[repr(usize)]
pub enum Gen {
    Eden = 0,
    Survivor = 1,
    Old = 2,
}

impl ::std::fmt::Debug for Region {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        write!(f, "Region({:?})", self.start())
    }
}

impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        self as *const _ == other as *const _
    }
}

