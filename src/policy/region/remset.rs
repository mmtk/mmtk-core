use super::REGIONS_IN_HEAP;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use super::*;
use util::constants::*;
use util::*;
use plan::selected_plan::PLAN;
use policy::space::Space;
use vm::*;


pub struct RemSet {
    prts: Box<[Option<Box<PerRegionTable>>; REGIONS_IN_HEAP]>,
}



impl RemSet {
    pub fn new() -> Self {
        Self {
            prts: unsafe { ::std::mem::transmute(box [0usize; REGIONS_IN_HEAP]) }
        }
    }


    fn get_per_region_table(&self, region: Region) -> &'static PerRegionTable {
        let index = region.heap_index();
        let entry: &AtomicPtr<PerRegionTable> = {
            let r: &Option<Box<PerRegionTable>> = &self.prts[index];
            debug_assert!(::std::mem::size_of::<Option<Box<PerRegionTable>>>() == ::std::mem::size_of::<AtomicPtr<PerRegionTable>>());
            unsafe { ::std::mem::transmute(r) }
        };
        let mut ptr = entry.load(Ordering::SeqCst);
        if ptr == 0 as _ {
            let new_table = Box::into_raw(box PerRegionTable::new(region));
            let old_table = entry.compare_and_swap(0 as _, new_table, Ordering::SeqCst);
            if old_table == 0 as _ {
                ptr = new_table;
            } else {
                // Drop this prt
                let _prt = unsafe { Box::from_raw(new_table) };
                ptr = old_table;
            }
        }
        unsafe { &*ptr }
    }

    pub fn add_card(&self, card: Card) {
        // debug_assert!(Region::of(card.0).committed);
        let prt = self.get_per_region_table(Region::of(card.0));
        prt.add_card(card);
    }

    pub fn contains_card(&self, card: Card) -> bool {
        // debug_assert!(Region::of(card.0).committed);
        let prt = self.get_per_region_table(Region::of(card.0));
        prt.contains_card(card)
    }

    pub fn clear_cards_in_collection_set(&mut self) {
        for prt in self.prts.iter_mut() {
            if prt.is_some() {
                if prt.as_ref().unwrap().region_in_cset() {
                    *prt = None;
                } else {
                    prt.as_ref().unwrap().clean_los_cards();
                }
            }
        }
    }

    #[inline(always)]
    pub fn iterate<F: Fn(Card)>(&self, f: F) {
        for prt in self.prts.iter() {
            if let Some(prt) = prt {
                if !prt.region_in_cset() {
                    prt.iterate(&f)
                }
            }
        }
    }
}

impl ::std::fmt::Debug for RemSet {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        writeln!(formatter, "<remset>")
    }
}


struct PerRegionTable {
    pub region: Address,
    pub data: Box<[AtomicUsize; CARDS_IN_REGION / BITS_IN_WORD]>,
}

impl PerRegionTable {
    fn new(region: Region) -> Self {
        Self {
            region: region.0,
            data: unsafe { ::std::mem::transmute(box [0usize; CARDS_IN_REGION / BITS_IN_WORD]) }
        }
    }

    fn get_entry(&self, card: Card) -> (&AtomicUsize, usize) {
        let index = (card.0.as_usize() & REGION_MASK) >> LOG_BYTES_IN_CARD;
        // const BYTES_IN_USIZE: usize = ::std::mem::size_of::<usize>();
        (&self.data[index >> LOG_BITS_IN_WORD], index & (BITS_IN_WORD - 1))
    }

    fn add_card(&self, card: Card) {
        let (entry, offset) = self.get_entry(card);
        entry.fetch_or(1 << offset, Ordering::SeqCst);
    }

    fn contains_card(&self, card: Card) -> bool {
        let (entry, offset) = self.get_entry(card);
        (entry.load(Ordering::SeqCst) & (1 << offset)) != 0
    }

    fn remove_card(&self, card: Card) {
        let (entry, offset) = self.get_entry(card);
        entry.fetch_and(!(1 << offset), Ordering::SeqCst);
    }

    #[cfg(not(feature = "g1"))]
    fn clean_los_cards(&self) {
        unimplemented!()
    }

    #[cfg(feature = "g1")]
    fn clean_los_cards(&self) {
        if PLAN.los.address_in_space(self.region) {
            self.iterate(&|card| {
                let o = unsafe { VMObjectModel::get_object_from_start_address(card.0) };
                if !PLAN.los.is_live(o) {
                    self.remove_card(card);
                }
            })
        }
    }

    #[cfg(not(feature = "g1"))]
    fn region_in_cset(&self) -> bool {
        unimplemented!()
    }

    #[cfg(feature = "g1")]
    fn region_in_cset(&self) -> bool {
        if PLAN.region_space.address_in_space(self.region) {
            Region::of(self.region).relocate
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn iterate<F: Fn(Card)>(&self, f: &F) {
        for i in 0..self.data.len() {
            let val = self.data[i].load(Ordering::SeqCst);
            if val != 0 {
                for j in 0..BITS_IN_WORD {
                    if (val & (1 << j)) != 0 {
                        // This card is remembered
                        let index = (i << LOG_BITS_IN_WORD) + j;
                        debug_assert!(index < CARDS_IN_REGION);
                        let card = self.region + (index << LOG_BYTES_IN_CARD);
                        debug_assert!(Card::align(card) == card);
                        debug_assert!(card >= self.region);
                        debug_assert!(card < (self.region + BYTES_IN_REGION), "{:?} {:?} {:?} {:?}", index, CARDS_IN_REGION, card, self.region.0);
                        f(Card(card))
                    }
                }
            }
        }
    }
}