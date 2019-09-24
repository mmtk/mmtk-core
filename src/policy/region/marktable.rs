use super::*;
use util::constants::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use util::*;
use vm::*;

const BITS_IN_MARK_TABLE: usize = BYTES_IN_REGION / BYTES_IN_WORD;
const MARK_TABLE_ENTRIES: usize = BITS_IN_MARK_TABLE / constants::BITS_IN_WORD;

pub struct MarkTable {
    data: [usize; MARK_TABLE_ENTRIES],
}

impl MarkTable {
    pub fn clear(&mut self) {
        VMMemory::zero(Address::from_ptr(self as _), ::std::mem::size_of::<Self>());
    }

    #[inline(always)]
    fn get_entry_for_address(&self, addr: Address) -> (usize, usize) {
        debug_assert!(!addr.is_zero());
        let diff = addr.as_usize() & REGION_MASK;
        let bit_index = diff >> LOG_BYTES_IN_WORD;
        let index = bit_index >> LOG_BITS_IN_WORD;
        let offset = bit_index & (BITS_IN_WORD - 1);
        (index, offset)
    }

    #[inline(always)]
    fn get_entry(&self, obj: ObjectReference) -> (usize, usize) {
        debug_assert!(!obj.is_null());
        let addr = VMObjectModel::ref_to_address(obj);
        self.get_entry_for_address(addr)
    }

    #[inline(always)]
    fn get_atomic_element(&self, index: usize) -> &AtomicUsize {
        unsafe {
            // let r: &usize = &self.data[index];
            let r: &usize = &*self.data.as_ptr().offset(index as isize);
            {
                let addr = Address::from_usize(r as *const _ as usize);
                use policy::space::Space;
                debug_assert!(::plan::g1::PLAN.region_space.address_in_space(addr));
                debug_assert!(Region::align(addr) == ::util::alloc::embedded_meta_data::get_metadata_base(addr));
            }
            ::std::mem::transmute(r)
        }
    }
    
    #[inline(always)]
    pub fn mark(&self, obj: ObjectReference, atomic: bool) -> bool {
        let (index, offset) = self.get_entry(obj);
        debug_assert!(index < self.data.len(), "{:?} {} {}", VMObjectModel::ref_to_address(obj), index, offset);
        let entry = self.get_atomic_element(index);
        let mask = 1usize << offset;
        if atomic {
            let old_value = entry.fetch_or(mask, Ordering::Relaxed);
            (old_value & mask) == 0
        } else {
            let value = entry.load(Ordering::Relaxed);
            if (value & mask) != 0 {
                return true
            }
            entry.store(value | mask, Ordering::Relaxed);
            true
        }
    }

    #[inline(always)]
    fn test(&self, a: Address) -> bool {
        let (index, offset) = self.get_entry_for_address(a);
        let entry = self.get_atomic_element(index);
        let mask = 1 << offset;
        let value = entry.load(Ordering::Relaxed);
        (value & mask) != 0
    }

    #[inline(always)]
    pub fn is_marked(&self, o: ObjectReference) -> bool {
        self.test(VMObjectModel::ref_to_address(o))
    }
    
    #[inline(always)]
    #[cfg(not(feature="jikesrvm"))]
    pub fn block_start(&self, region: RegionRef, start: Address, end: Address) -> Option<Address> {
        unimplemented!()
    }

    #[inline(always)]
    #[cfg(feature="jikesrvm")]
    pub fn block_start(&self, region: RegionRef, start: Address, end: Address) -> Option<Address> {
        // assert!(start < region.next_cursor);
        // Find first slot of a object
        let region_end = region.next_cursor;
        let mut cursor = start;
        let limit = end + 24usize;
        let limit = if limit > region_end { region_end } else { limit };
        while cursor < limit {
            if self.test(cursor) {
                use ::vm::jikesrvm::java_header::TIB_OFFSET;
                let object_address = cursor + (-TIB_OFFSET);
                let object = unsafe { object_address.to_object_reference() };
                debug_assert!(VMObjectModel::ref_to_address(object) == cursor);
                let obj_start = VMObjectModel::object_start_ref(object);
                if obj_start >= start && obj_start < end {
                    return Some(obj_start);
                } else if obj_start >= end {
                    break;
                }
            }
            cursor = cursor + BYTES_IN_ADDRESS;
        }
        return None;
    }

    #[inline(always)]
    #[cfg(not(feature="jikesrvm"))]
    pub fn iterate<F: Fn(ObjectReference)>(&self, start: Address, end: Address, f: F) {
        unimplemented!()
    }

}

impl ::std::fmt::Debug for MarkTable {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        writeln!(formatter, "<marktable>")
    }
}