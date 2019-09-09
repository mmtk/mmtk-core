use super::card::*;
use util::heap::layout::vm_layout_constants::*;
use util::*;

static mut CARD_TABLE: CardTable = CardTable {
    table: [CardState::NotDirty; CARDS_IN_HEAP]
};

static mut CARD_HOTNESS_TABLE: [u8; CARDS_IN_HEAP] = [0; CARDS_IN_HEAP];
const HOTNESS_THRESHOLD: u8 = 5;

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
        (card.0 - HEAP_START) >> LOG_BYTES_IN_CARD
    }

    pub fn inc_hotness(card: Card) -> bool {
        let index = Self::get_index(card);
        unsafe {
            if CARD_HOTNESS_TABLE[index] >= HOTNESS_THRESHOLD {
                return true;
            }
            CARD_HOTNESS_TABLE[index] += 1;
            false
        }
    }

    pub fn clear_hotness(card: Card) {
        let index = Self::get_index(card);
        unsafe {
            CARD_HOTNESS_TABLE[index] = 0;
        }
    }
}

impl CardTable {
    #[inline(always)]
    pub fn get_entry(&self, addr: Address) -> CardState {
        debug_assert!(addr >= HEAP_START && addr < HEAP_END);
        self.table[(addr - HEAP_START) >> LOG_BYTES_IN_CARD]
    }

    #[inline(always)]
    pub fn set_entry(&mut self, addr: Address, state: CardState) {
        debug_assert!(addr >= HEAP_START && addr < HEAP_END);
        self.table[(addr - HEAP_START) >> LOG_BYTES_IN_CARD] = state;
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
