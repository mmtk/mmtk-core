use super::generic_freelist::*;
use ::util::constants::*;
use ::util::Address;
use ::util::conversions;
use std::mem;
use ::plan::selected_plan;
use ::plan::plan::Plan;

#[derive(Debug)]
pub struct IntArrayFreeList {
    pub head: i32,
    pub heads: i32,
    pub table: Option<Vec<i32>>,
    parent: Option<&'static IntArrayFreeList>,
}

impl GenericFreeList for IntArrayFreeList {
    fn head(&self) -> i32 {
        self.head
    }
    fn heads(&self) -> i32 {
        self.heads
    }
    fn get_entry(&self, index: i32) -> i32 {
        self.table()[index as usize]
    }
    fn set_entry(&mut self, index: i32, value: i32) {
        self.table_mut()[index as usize] = value;
    }
}

impl IntArrayFreeList {
    pub fn new(units: usize, grain: i32, heads: usize) -> Self {
        debug_assert!(units <= MAX_UNITS as usize && heads <= MAX_HEADS as usize);
        // allocate the data structure, including space for top & bottom sentinels
        let len = (units + 1 + heads) << 1;
        let mut iafl = IntArrayFreeList {
            head: -1,
            heads: heads as _,
            table: Some(vec![0; len]), // len=2052
            parent: None,
        };
        iafl.initialize_heap(units as _, grain);
        iafl
    }
    pub fn from_parent(parent: &IntArrayFreeList, ordinal: i32) -> Self {
        let iafl = IntArrayFreeList {
            head: -(1 + ordinal),
            heads: parent.heads,
            table: None,
            parent: Some(unsafe { mem::transmute(parent) }),
        };
        debug_assert!(-iafl.head <= iafl.heads);
        iafl
    }
    fn table(&self) -> &Vec<i32> {
        match self.parent {
            Some(p) => p.table(),
            None => self.table.as_ref().unwrap()
        }
    }
    #[allow(mutable_transmutes)]
    fn table_mut(&mut self) -> &mut Vec<i32> {
        match self.parent {
            Some(p) => {
                let parent_mut: &mut Self = unsafe { mem::transmute(p) };
                parent_mut.table_mut()
            },
            None => self.table.as_mut().unwrap()
        }
    }
    pub fn resize_parent_freelist(&mut self, units: usize, grain: i32) {
        // debug_assert!(self.parent.is_none() && !selected_plan::PLAN.is_initialized());
        *self.table_mut() = vec![0; (units + 1 + self.heads as usize) << 1];
        self.initialize_heap(units as _, grain);
    }
    // pub fn resize_child_freelist(&mut self) {
    //     debug_assert!(self.parent.is_some() && !Plan.isInitialized());
    //     // self.table_mut() = Taself.parent.unwrap().table()
    // }
}