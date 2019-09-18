use super::card::*;
use util::heap::layout::vm_layout_constants::*;
use util::*;

static mut CARD_TABLE: CardTable = CardTable {
    table: [CardState::NotDirty; CARDS_IN_HEAP]
};

static mut CARD_HOTNESS_TABLE: [u8; CARDS_IN_HEAP] = [0; CARDS_IN_HEAP];
const HOTNESS_THRESHOLD: u8 = 4;

#[inline(always)]
pub fn get() -> &'static mut CardTable {
    unsafe { &mut CARD_TABLE }
}

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CardState {
    Young = 0,
    NotDirty = 1,
    Dirty = 2,
}

pub struct CardTable {
    table: [CardState; CARDS_IN_HEAP]
}

impl CardTable {
    fn get_index(card: Card) -> usize {
        (card.start() - HEAP_START) >> LOG_BYTES_IN_CARD
    }

    pub fn inc_hotness(card: Card) -> bool {
        let index = Self::get_index(card);
        if Self::get_hot_entry(index) >= HOTNESS_THRESHOLD {
            return true;
        }
        Self::inc_hot_entry(index);
        false
    }

    // pub fn clear_hotness(card: Card) {
    //     let index = Self::get_index(card);
    //     unsafe {
    //         CARD_HOTNESS_TABLE[index] = 0;
    //     }
    // }

    #[inline(always)]
    fn inc_hot_entry(index: usize) {
        unsafe {
            debug_assert!(index < CARD_HOTNESS_TABLE.len());
            *CARD_HOTNESS_TABLE.get_unchecked_mut(index) += 1;
        }
    }

    #[inline(always)]
    fn get_hot_entry(index: usize) -> u8 {
        unsafe {
            debug_assert!(index < CARD_HOTNESS_TABLE.len());
            *CARD_HOTNESS_TABLE.get_unchecked(index)
        }
    }

    pub fn clear_all_hotness_par(id: usize, num_workers: usize) {
        let size = (CARDS_IN_HEAP + num_workers - 1) / num_workers;
        let start = size * id;
        let limit = size * (id + 1);
        let limit = if limit > CARDS_IN_HEAP { CARDS_IN_HEAP } else { limit };
        let size = limit - start;
        unsafe {
            let table: *mut [u8; CARDS_IN_HEAP] = &mut CARD_HOTNESS_TABLE;
            let table_ptr = table as usize as *mut u8;
            ::libc::memset(table_ptr.add(start) as usize as *mut _, 0, size);
        }
    }
}

impl CardTable {
    #[inline(always)]
    pub fn get_entry(&self, addr: Address) -> CardState {
        debug_assert!(addr >= HEAP_START && addr < HEAP_END);
        // self.table[(addr - HEAP_START) >> LOG_BYTES_IN_CARD]
        let index = (addr - HEAP_START) >> LOG_BYTES_IN_CARD;
        debug_assert!(index < self.table.len());
        unsafe {
            *self.table.get_unchecked(index)
        }
    }

    #[inline(always)]
    pub fn set_entry(&mut self, addr: Address, state: CardState) {
        debug_assert!(addr >= HEAP_START && addr < HEAP_END);
        // self.table[(addr - HEAP_START) >> LOG_BYTES_IN_CARD] = state;
        let index = (addr - HEAP_START) >> LOG_BYTES_IN_CARD;
        debug_assert!(index < self.table.len());
        unsafe {
            *self.table.get_unchecked_mut(index) = state;
        }
    }

    pub fn assert_all_cards_are_not_marked(&self) {
        assert!(cfg!(debug_assertions));
        for i in 0..CARDS_IN_HEAP {
            assert!(self.table[i] == CardState::NotDirty);
            unsafe {
                assert!(CARD_HOTNESS_TABLE[i] == 0);
            }
        }
    }
}
