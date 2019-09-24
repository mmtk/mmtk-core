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
pub struct Card(Address);

impl Card {
    #[inline]
    pub fn start(&self) -> Address {
        self.0
    }

    #[inline]
    pub fn align(address: Address) -> Address {
        unsafe { Address::from_usize(address.to_address().0 & !CARD_MASK) }
    }

    #[inline]
    pub unsafe fn unchecked(a: Address) -> Self {
        debug_assert!(Self::align(a) == a);
        Self(a)
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

    fn scan_g1<Closure: Fn(ObjectReference)>(&self, region: RegionRef, start: Address, limit: Address, cl: Closure) {
        let region = region.get_mut();
        let mut cursor = match region.card_offset_table.block_start(*self) {
            Some(a) => a,
            None => return,
        };
        // assert!(start <= cursor);
        while cursor < limit {
            let object = match unsafe { super::get_object_from_start_address(cursor, limit) } {
                Some(o) => o.1,
                None => break,
            };
            // assert!(cursor <= start_ref);
            // assert!(start_ref < limit);
            cl(object);
            cursor = VMObjectModel::get_object_end_address(object);
            // if start_ref >= start {
            //     if start_ref < limit {
            //         cl(object);
            //     }
            // }
        }
    }

    #[inline(always)]
    #[cfg(feature = "g1")]
    pub fn linear_scan<Closure: Fn(ObjectReference)>(&self, cl: Closure, mark_dead: bool) {
        // if CLEAR_HOTNESS {
        //     CardTable::clear_hotness(*self);
        // }
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
            debug_assert!(region.committed, "Invalid region {:?} in chunk {:?}", region.start(), ::util::alloc::embedded_meta_data::get_metadata_base(region.start()));
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