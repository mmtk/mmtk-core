use super::ToAddress;
use util::*;
use super::region::*;
use super::cardtable;
use super::cardtable::CardTable;
use util::heap::layout::vm_layout_constants::*;
use vm::*;
use plan::selected_plan::PLAN;
use policy::space::Space;
use util::alloc::bumpallocator;

pub const LOG_BYTES_IN_CARD: usize = 9;
pub const BYTES_IN_CARD: usize = 1 << LOG_BYTES_IN_CARD;
pub const CARDS_IN_HEAP: usize = (HEAP_END.as_usize() - HEAP_START.as_usize()) / BYTES_IN_CARD;
pub const CARDS_IN_REGION: usize = BYTES_IN_REGION / BYTES_IN_CARD;
pub const CARD_MASK: usize = BYTES_IN_CARD - 1;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd)]
pub struct Card(pub Address);

impl Card {
    #[inline]
    pub fn align(address: Address) -> Address {
        unsafe { Address::from_usize(address.to_address().0 & !CARD_MASK) }
    }

    #[inline]
    pub fn of<TA: ToAddress>(ta: TA) -> Self {
        Self(Self::align(ta.to_address()))
    }

    pub fn get_region(&self) -> Address {
        super::Region::align(self.0)
    }

    #[inline]
    pub fn get_state(&self) -> cardtable::CardState {
        cardtable::get().get_entry(self.0)        
    }

    #[inline]
    pub fn inc_hotness(&self) -> bool {
        CardTable::inc_hotness(*self)
    }

    #[inline]
    pub fn set_state(&self, s: cardtable::CardState) {
        cardtable::get().set_entry(self.0, s);
    }
    
    #[inline(always)]
    #[cfg(not(feature = "g1"))]
    pub fn linear_scan<Closure: Fn(ObjectReference)>(&self, cl: Closure, mark_dead: bool) {
        unimplemented!()
    }
    
    #[inline(never)]
    #[allow(dead_code)]
    #[cfg(feature = "g1")]
    fn scan_g1<Closure: Fn(ObjectReference)>(&self, mut region: Region, start: Address, limit: Address, cl: Closure) {
        // println!("Scan card {:?} in {:?} {:?}", start, region, region.relocate);
        // println!("limit {:?}", limit);
        let mut cursor = region.prev_mark_table().block_start(start, limit);
        let mut should_update_cot = cursor < start;
        // println!("block_start {:?}", cursor);
        while cursor < limit {
            // println!("  cursor={:?}", cursor);
            let object = match unsafe { get_object_from_start_address(cursor, limit) } {
                Some(o) => o,
                None => break,
            };
            
            // println!("  object={:?}", object);
            let start_ref = VMObjectModel::object_start_ref(object);
            // let start_ref = object.to_address() + (-::vm::jikesrvm::java_header::OBJECT_REF_OFFSET);
            // debug_assert!(unsafe { VMObjectModel::get_object_from_start_address(cursor) } == object);
            // println!("  start_ref={:?}", start_ref);
            if start_ref >= limit {
                break;
            }
            if ::util::alloc::bumpallocator::tib_is_zero(object) {
                return
            }
            cursor = VMObjectModel::get_object_end_address(object);
            debug_assert!(cursor == start_ref + VMObjectModel::get_current_size(object));
            if start_ref >= start && start_ref < limit {
                if should_update_cot {
                    should_update_cot = false;
                    let cot_index = (start - region.0) >> LOG_BYTES_IN_CARD;
                    region.card_offset_table[cot_index] = start_ref;
                }

                cl(object);
            }
        }
        // println!("Scan card {:?} Finish", start);
    }
    
    #[inline(always)]
    #[cfg(feature = "g1")]
    pub fn linear_scan<Closure: Fn(ObjectReference)>(&self, cl: Closure, mark_dead: bool) {
        CardTable::clear_hotness(*self);
        use plan::plan::Plan;
        if !::plan::selected_plan::PLAN.is_mapped_address(self.0) {
            return
        }
        if PLAN.region_space.address_in_space(self.0) {
            let region = Region::of(self.0);
            if !region.committed {
                return
            }
            if region.relocate {
                return
            }
            debug_assert!(region.committed, "Invalid region {:?} in chunk {:?}", region.0, ::util::alloc::embedded_meta_data::get_metadata_base(region.0));
            // region.prev_mark_table().iterate(self.0, self.0 + BYTES_IN_CARD, cl);
            self.scan_g1(region, self.0, self.0 + BYTES_IN_CARD, cl);
        } else if PLAN.los.address_in_space(self.0) {
            let o = unsafe { VMObjectModel::get_object_from_start_address(self.0) };
            if PLAN.los.is_live(o) {
                cl(o);
            }
        } else if PLAN.versatile_space.address_in_space(self.0) {
            bumpallocator::linear_scan(self.0, self.0 + BYTES_IN_CARD, |obj| {
                // cl(obj);
                if mark_dead {
                    if PLAN.versatile_space.is_marked(obj) && !is_dead(obj) {
                        cl(obj);
                    } else {
                        mark_as_dead(obj);
                    }
                } else {
                    if !is_dead(obj) {
                        cl(obj);
                    }
                }
            });
        } else {
            // Do nothing...
        }
    }
}

const DEATH_BIT: u8 = 0b1000;

fn mark_as_dead(object: ObjectReference) {
    let value = VMObjectModel::read_available_byte(object);
    VMObjectModel::write_available_byte(object, value | DEATH_BIT);
}

fn is_dead(object: ObjectReference) -> bool {
    let value = VMObjectModel::read_available_byte(object);
    (value & DEATH_BIT) == DEATH_BIT
}

impl ::std::ops::Deref for Card {
    type Target = Address;
    fn deref(&self) -> &Address {
        &self.0
    }
}

#[cfg(feature="jikesrvm")]
unsafe fn get_object_from_start_address(start: Address, limit: Address) -> Option<ObjectReference> {
    // trace!("ObjectModel.get_object_from_start_address");
    let mut _start = start;
    if _start >= limit {
            return None;
        }
    /* Skip over any alignment fill */
    while _start.load::<usize>() == ::vm::jikesrvm::java_header::ALIGNMENT_VALUE {
        _start += ::std::mem::size_of::<usize>();
        if _start >= limit {
            return None;
        }
    }
    Some((_start + ::vm::jikesrvm::java_header::OBJECT_REF_OFFSET).to_object_reference())
}

#[cfg(not(feature="jikesrvm"))]
unsafe fn get_object_from_start_address(start: Address, limit: Address) -> Option<ObjectReference> {
    unimplemented!()
}