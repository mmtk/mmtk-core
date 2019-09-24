use util::*;
use super::*;
use vm::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct CardOffsetTable {
    table: [Address; CARDS_IN_REGION],
}

impl CardOffsetTable {
    #[inline(always)]
    fn index(&self, card: Card) -> usize {
        let region_start = Region::align(*card);
        let index = (*card - region_start) >> LOG_BYTES_IN_CARD;
        debug_assert!(index < self.table.len());
        index
    }
    
    #[inline(always)]
    pub fn set(&mut self, card: Card, start: Address) {
        let index = self.index(card);
        unsafe {
            *self.table.get_unchecked_mut(index) = start;
        }
    }

    #[inline(always)]
    fn get(&self, card: Card) -> Address {
        let index = self.index(card);
        unsafe {
            *self.table.get_unchecked(index)
        }
    }

    #[inline(always)]
    pub fn attempt(&mut self, card: Card, old: Address, new: Address) -> bool {
        let index = self.index(card);
        unsafe {
            let r: &mut Address = self.table.get_unchecked_mut(index);
            let a: &mut AtomicUsize = ::std::mem::transmute(r);
            a.compare_and_swap(old.as_usize(), new.as_usize(), Ordering::Relaxed) == old.as_usize()
        }
    }

    #[inline(always)]
    pub fn block_start(&mut self, card: Card) -> Option<Address> {
        let start = self.get(card);
        if start >= card.start() {
            Some(start)
        } else {
            // let region = Region::of(*card);
            // if region.prev_cursor <= *card && *card <= region.next_cursor {
            //     if let Some(start) = region.prev_mark_table().block_start(region, card.start(), card.start() + BYTES_IN_CARD) {
            //         self.set(card, start);
            //         Some(start)
            //     } else {
            //         None
            //     }
            // } else {
                self.block_start_slow(card, start)
            // }
        }  
    }

    #[inline(always)]
    fn block_start_slow(&mut self, card: Card, block_start: Address) -> Option<Address> {
        let mut cursor = block_start;
        let limit = card.start() + BYTES_IN_CARD;
        // let mut prev_card = Card::of(cursor);

        loop {
            {
                // let curr_card = Card::of(cursor);
                // if curr_card != prev_card {
                //     if self.get(curr_card) < curr_card.start() {
                //         self.set(curr_card, cursor);
                //     }
                //     prev_card = curr_card;
                // }
                // let card = Card::of(cursor);
                // if self.get(card) < card.start() {
                //     self.set(card, cursor)
                // }
            }
            let (start_ref, object) = match unsafe { super::get_object_from_start_address(cursor, limit) } {
                Some(o) => o,
                None => return None,
            };

            {
                // let curr_card = Card::of(start_ref);
                // if curr_card != prev_card {
                    
                //     if self.get(curr_card) < curr_card.start() {
                //         self.set(curr_card, start_ref);
                //     }
                //     prev_card = curr_card;
                // }
                // let card = Card::of(start_ref);
                // if self.get(card) < card.start() {
                //     self.set(card, start_ref)
                // }
            }
            if start_ref >= card.start() {
                let region_end = Region::of(*card).next_cursor;
                let mut c = card.start();
                let mut c_index = self.index(card);
                while c < region_end {
                    let bot_entry = unsafe { self.table.get_unchecked_mut(c_index) };
                    if *bot_entry < start_ref {
                        *bot_entry = start_ref
                    } else {
                        break;
                    }
                    c += BYTES_IN_CARD;
                    c_index += 1;
                }
                return Some(start_ref);
            }
            cursor = VMObjectModel::get_object_end_address(object);
            debug_assert!(cursor == start_ref + VMObjectModel::get_current_size(object));
        }
    }

    pub fn clear(&mut self) {
        for i in 0..CARDS_IN_REGION {
            // self.table[i] = unsafe { Address::zero() };
            self.table[i] = unsafe { Address::zero() };
        }
    }
}

