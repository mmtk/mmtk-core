use super::ToAddress;
use util::*;
use super::region::*;
use super::cardtable;
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
    pub fn set_state(&self, s: cardtable::CardState) {
        cardtable::get().set_entry(self.0, s);
    }
    
    #[inline(always)]
    #[cfg(not(feature = "g1"))]
    pub fn linear_scan<Closure: Fn(ObjectReference)>(&self, cl: Closure) {
        unimplemented!()
    }
    
    #[inline(never)]
    #[cfg(feature = "g1")]
    fn scan_g1<Closure: Fn(ObjectReference)>(&self, start: Address, limit: Address, cl: Closure) {
        let v = ::plan::plan::gc_in_progress();
        let mut cursor = Region::align(start);
        if v { println!("start {:?} limit {:?} region-limit {:?}", start, limit, Region::of(start).cursor); }
        while cursor < limit {
            // if ::plan::plan::gc_in_progress() {
            //     println!("Cursor: {:?} in region {:?}", cursor, Region::align(start));
            // }
            let object = unsafe { VMObjectModel::get_object_from_start_address(cursor) };
            let object_start = VMObjectModel::object_start_ref(object);
            if object_start >= limit {
                break
            }
            if object_start >= start {
                if v { println!(" - scan {:?}", object); }
                cl(object);
                if v { println!(" - scan {:?} end {:?}", object, VMObjectModel::get_object_end_address(object)); }
            }
            cursor = VMObjectModel::get_object_end_address(object);
        }
        if v { println!("finish {:?}", start); }
    }
    
    #[inline(always)]
    #[cfg(feature = "g1")]
    pub fn linear_scan<Closure: Fn(ObjectReference)>(&self, cl: Closure) {
        use plan::plan::Plan;
        if !::plan::selected_plan::PLAN.is_mapped_address(self.0) {
            return
        }
        if PLAN.region_space.address_in_space(self.0) {
            let region = Region::of(self.0);
            if !region.committed {
                return
            }
            debug_assert!(region.committed, "Invalid region {:?} in chunk {:?}", region.0, ::util::alloc::embedded_meta_data::get_metadata_base(region.0));
            // let cursor = 
            region.prev_mark_table().iterate(self.0, self.0 + BYTES_IN_CARD, cl);
            // self.scan_g1(self.0, self.0 + BYTES_IN_CARD, cl);
        } else if PLAN.los.address_in_space(self.0) {
            let o = unsafe { VMObjectModel::get_object_from_start_address(self.0) };
            // if PLAN.los.is_live(o) {
                cl(o)
            // }
        } else if PLAN.versatile_space.address_in_space(self.0) {
            bumpallocator::linear_scan(self.0, self.0 + BYTES_IN_CARD, cl);
        } else {
            // Do nothing...
        }
    }
}

impl ::std::ops::Deref for Card {
    type Target = Address;
    fn deref(&self) -> &Address {
        &self.0
    }
}