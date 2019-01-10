use util::alloc::embedded_meta_data;
use util::constants;
use util::Address;
use util::ObjectReference;
use std::sync::atomic::{AtomicUsize, Ordering};
use vm::{VMObjectModel, ObjectModel, VMMemory, Memory};
use std::ops::{Deref, DerefMut};

pub const LOG_PAGES_IN_REGION: usize = 6;
pub const PAGES_IN_REGION: usize = 1 << LOG_PAGES_IN_REGION; // 256
pub const LOG_BYTES_IN_REGION: usize = LOG_PAGES_IN_REGION + constants::LOG_BYTES_IN_PAGE as usize;
pub const BYTES_IN_REGION: usize = 1 << LOG_BYTES_IN_REGION;//BYTES_IN_PAGE * PAGES_IN_REGION; // 1048576
const REGION_MASK: usize = BYTES_IN_REGION - 1;// 0..011111111111

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
        self.to_address()
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Region(pub Address);

impl Region {
    #[inline]
    pub fn align(address: Address) -> Address {
        unsafe { Address::from_usize(address.to_address().0 & !REGION_MASK) }
    }

    #[inline]
    pub fn of<TA: ToAddress>(a: TA) -> Region {
        Region(Self::align(a.to_address()))
    }

    #[inline]
    pub fn index(&self) -> usize {
        let chunk = embedded_meta_data::get_metadata_base(self.0);
        let region_start = chunk + METADATA_BYTES_PER_CHUNK;
        let index = (self.0 - region_start) >> LOG_BYTES_IN_REGION;
        index
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
    pub mark_table: MarkBitMap,
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
    pub fn clear(&mut self) {
        self.mark_table.clear();
        VMMemory::zero(Address::from_ptr(self as _), ::std::mem::size_of::<Self>());
    }
}



const OBJECT_LIVE_SHIFT: usize = constants::LOG_BYTES_IN_INT as usize; // 4 byte resolution
const LOG_BIT_COVERAGE: usize = OBJECT_LIVE_SHIFT;
const LOG_LIVE_COVERAGE: usize = LOG_BIT_COVERAGE + constants::LOG_BITS_IN_BYTE as usize;
const WORD_SHIFT_MASK: usize = (1 << constants::LOG_BITS_IN_WORD) - 1;
const MARK_TABLE_SIZE: usize = METADATA_BYTES_PER_REGION - ::std::mem::size_of::<MetaData>();

#[repr(C)]
#[derive(Debug)]
pub struct MarkBitMap {
    table: ()
}

impl MarkBitMap {
    #[inline]
    fn table(&self) -> Address {
        unsafe { ::std::mem::transmute(&self.table) }
    }

    #[inline]
    pub fn is_marked(&self, object: ObjectReference) -> bool {
        self.live_bit_set(VMObjectModel::object_start_ref(object))
    }

    #[inline]
    pub fn test_and_mark(&self, object: ObjectReference) -> bool {
        self.set_live_bit(VMObjectModel::object_start_ref(object), true)
    }

    #[inline]
    pub fn write_mark_state(&self, object: ObjectReference) {
        self.set_live_bit(VMObjectModel::object_start_ref(object), false);
    }

    #[inline]
    fn set_live_bit(&self, address: Address, atomic: bool) -> bool {
        let live_word = self.get_live_word(address);
        let mask = Self::get_mask(address);
        let old_value = if atomic {
            live_word.fetch_or(mask, Ordering::Relaxed)
        } else {
            let old_value = live_word.load(Ordering::Relaxed);
            live_word.store(old_value | mask, Ordering::Relaxed);
            old_value
        };
        (old_value & mask) != mask
    }

    #[inline]
    fn live_bit_set(&self, address: Address) -> bool {
      let live_word = self.get_live_word(address);
      let mask = Self::get_mask(address);
      let value = live_word.load(Ordering::Relaxed);
      (value & mask) == mask
    }

    #[inline]
    fn get_mask(address: Address) -> usize {
        let shift = (address.0 >> OBJECT_LIVE_SHIFT) & WORD_SHIFT_MASK;
        1 << shift
    }
    
    #[inline]
    pub fn get_live_word_offset(&self, address: Address, log_coverage: usize, log_align: usize) -> usize {
        ((address.0 & REGION_MASK) >> (log_coverage + log_align)) << log_align
    }

    #[inline]
    fn get_live_word_address(&self, address: Address) -> Address {
        self.table() + self.get_live_word_offset(address, LOG_LIVE_COVERAGE, constants::LOG_BYTES_IN_WORD as _)
    }

    #[inline]
    fn get_live_word(&self, address: Address) -> &AtomicUsize {
        let address = self.get_live_word_address(address);
        debug_assert!(address >= self.table());
        debug_assert!(address < ((self.table() + MARK_TABLE_SIZE)));
        unsafe { ::std::mem::transmute(address) }
    }

    #[inline]
    pub fn clear(&mut self) {
        VMMemory::zero(self.table(), MARK_TABLE_SIZE);
        // unimplemented!();
        // let start = EmbeddedMetaData.getMetaDataBase(region).plus(MARKING_METADATA_START);
        // VMMemory::zero(start, Extent.fromIntZeroExtend(MARKING_METADATA_EXTENT));
    }
}



