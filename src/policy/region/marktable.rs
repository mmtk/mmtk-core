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
    pub fn block_start(&self, start: Address, end: Address) -> Address {
        unimplemented!()
    }

    #[inline(always)]
    #[cfg(feature="jikesrvm")]
    pub fn block_start(&self, region: RegionRef, start: Address, end: Address) -> Address {
        // let mut region = Region::of(start);
        let cot_index = (start - region.start()) >> LOG_BYTES_IN_CARD;
        debug_assert!(cot_index < region.card_offset_table.len());
        let addr = unsafe { *region.card_offset_table.get_unchecked(cot_index) };
        if addr >= start {
            debug_assert!(addr < end);
            return addr;
        }
        // Find first slot of a object
        let region_end = region.cursor;
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
                    // Update COT
                    // region.get_mut().card_offset_table[cot_index] = obj_start;
                    debug_assert!(cot_index < region.card_offset_table.len());
                    unsafe {
                        *region.get_mut().card_offset_table.get_unchecked_mut(cot_index) = obj_start;
                    }
                    return obj_start;
                } else if obj_start >= end {
                    break;
                }
            }
            cursor = cursor + BYTES_IN_ADDRESS;
        }
        return end;
    }

    #[inline(always)]
    #[cfg(not(feature="jikesrvm"))]
    pub fn iterate<F: Fn(ObjectReference)>(&self, start: Address, end: Address, f: F) {
        unimplemented!()
    }

    #[inline(always)]
    #[cfg(feature="jikesrvm")]
    pub fn iterate<F: Fn(ObjectReference)>(&self, start: Address, end: Address, f: F) {
        let region_end = Region::of(start).cursor;
        // Find first slot of a object
        let mut cursor = start;
        let limit = end + 24usize;
        let limit = if limit > region_end { region_end } else { limit };
        while cursor < limit {
            if self.test(cursor) {
                // let object = unsafe { VMObjectModel::get_object_from_start_address(cursor) };
                // debug_assert!(VMObjectModel::object_start_ref(object) == cursor);
                use ::vm::jikesrvm::java_header::TIB_OFFSET;
                let object_address = cursor + (-TIB_OFFSET);
                let object = unsafe { object_address.to_object_reference() };
                debug_assert!(VMObjectModel::ref_to_address(object) == cursor);
                let obj_start = VMObjectModel::object_start_ref(object);
                if obj_start >= start && obj_start < end {
                    f(object);
                } else if obj_start >= end {
                    break;
                }
            }
            cursor = cursor + BYTES_IN_ADDRESS;
        }
    }

    #[inline(never)]
    pub fn zero_dead_memory(&self, region: Address) {
        debug_assert!(Region::of(region).committed);
        let limit = Region::of(region).cursor;
        let mut cursor = region;
        while cursor < limit {
            if self.test(cursor) {
                let object = unsafe { VMObjectModel::get_object_from_start_address(cursor) };
                debug_assert!(VMObjectModel::object_start_ref(object) == cursor);
                let end = VMObjectModel::get_object_end_address(object);
                cursor = end;
            } else {
                unsafe {
                    cursor.store(0x0usize);
                }
                cursor = cursor + BYTES_IN_ADDRESS;
            }
        }
    }
}

impl ::std::fmt::Debug for MarkTable {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        writeln!(formatter, "<marktable>")
    }
}