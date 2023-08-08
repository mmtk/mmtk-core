use super::freelist::*;
use std::{mem, ptr::NonNull};

#[derive(Debug)]
pub struct IntArrayFreeList {
    pub head: i32,
    pub heads: i32,
    pub table: Option<Vec<i32>>,
    parent: Option<NonNull<IntArrayFreeList>>,
}

unsafe impl Send for IntArrayFreeList {}
unsafe impl Sync for IntArrayFreeList {}

impl FreeList for IntArrayFreeList {
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
    pub(crate) fn get_ordinal(&self) -> i32 {
        -self.head - 1
    }
    fn table(&self) -> &Vec<i32> {
        match self.parent {
            Some(p) => unsafe { p.as_ref().table() },
            None => self.table.as_ref().unwrap(),
        }
    }

    // FIXME: We need a safe implementation

    fn table_mut(&mut self) -> &mut Vec<i32> {
        match self.parent {
            Some(mut p) => unsafe { p.as_mut().table_mut() },
            None => self.table.as_mut().unwrap(),
        }
    }
    pub fn resize_freelist(&mut self, units: usize, grain: i32) {
        // debug_assert!(self.parent.is_none() && !selected_plan::PLAN.is_initialized());
        *self.table_mut() = vec![0; (units + 1 + self.heads as usize) << 1];
        self.initialize_heap(units as _, grain);
    }
}

#[cfg(test)]
mod tests {
    use super::FreeList;
    use super::*;

    const LIST_SIZE: usize = 5;
    const TOP_SENTINEL: i32 = -1;
    const FIRST_UNIT: i32 = 0;
    const LAST_UNIT: i32 = LIST_SIZE as i32 - 1;
    const BOTTOM_SENTINEL: i32 = LIST_SIZE as i32;

    #[test]
    #[allow(clippy::cognitive_complexity)] // extensive checks, and it doesn't matter for tests
    fn new_free_list_grain1() {
        let l = IntArrayFreeList::new(LIST_SIZE, 1, 1);
        assert_eq!(l.head(), TOP_SENTINEL);

        assert_eq!(l.get_prev(TOP_SENTINEL), LAST_UNIT);
        assert_eq!(l.get_next(TOP_SENTINEL), FIRST_UNIT);

        assert_eq!(l.get_size(FIRST_UNIT), 1);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), -1);
        assert_eq!(l.get_right(FIRST_UNIT), 1);
        assert_eq!(l.get_next(FIRST_UNIT), 1);
        assert!(l.is_free(FIRST_UNIT));
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(!l.is_multi(FIRST_UNIT));

        assert_eq!(l.get_size(1), 1);
        assert_eq!(l.get_left(1), 0);
        assert_eq!(l.get_prev(1), 0);
        assert_eq!(l.get_right(1), 2);
        assert_eq!(l.get_next(1), 2);
        assert!(l.is_free(1));
        assert!(l.is_coalescable(1));
        assert!(!l.is_multi(1));

        assert_eq!(l.get_size(LAST_UNIT), 1);
        assert_eq!(l.get_left(LAST_UNIT), LAST_UNIT - 1);
        assert_eq!(l.get_prev(LAST_UNIT), LAST_UNIT - 1);
        assert_eq!(l.get_right(LAST_UNIT), BOTTOM_SENTINEL);
        assert_eq!(l.get_next(LAST_UNIT), -1);
        assert!(l.is_free(LAST_UNIT));
        assert!(l.is_coalescable(LAST_UNIT));
        assert!(!l.is_multi(LAST_UNIT));

        assert_eq!(l.get_prev(BOTTOM_SENTINEL), BOTTOM_SENTINEL);
        assert_eq!(l.get_next(BOTTOM_SENTINEL), BOTTOM_SENTINEL);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // extensive checks, and it doesn't matter for tests
    fn new_free_list_grain2() {
        let l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        assert_eq!(l.head(), TOP_SENTINEL);

        assert_eq!(l.get_prev(TOP_SENTINEL), LAST_UNIT);
        assert_eq!(l.get_next(TOP_SENTINEL), FIRST_UNIT);

        assert_eq!(l.get_size(FIRST_UNIT), 2);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), -1);
        assert_eq!(l.get_right(FIRST_UNIT), 2);
        assert_eq!(l.get_next(FIRST_UNIT), 2);
        assert!(l.is_free(FIRST_UNIT));
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(l.is_multi(FIRST_UNIT));

        assert_eq!(l.get_size(2), 2);
        assert_eq!(l.get_left(2), 0);
        assert_eq!(l.get_prev(2), 0);
        assert_eq!(l.get_right(2), 4);
        assert_eq!(l.get_next(2), 4);
        assert!(l.is_free(2));
        assert!(l.is_coalescable(2));
        assert!(l.is_multi(2));

        assert_eq!(l.get_size(LAST_UNIT), 1);
        assert_eq!(l.get_left(LAST_UNIT), LAST_UNIT - 2);
        assert_eq!(l.get_prev(LAST_UNIT), LAST_UNIT - 2);
        assert_eq!(l.get_right(LAST_UNIT), BOTTOM_SENTINEL);
        assert_eq!(l.get_next(LAST_UNIT), -1);
        assert!(l.is_free(LAST_UNIT));
        assert!(l.is_coalescable(LAST_UNIT));
        assert!(!l.is_multi(LAST_UNIT));

        assert_eq!(l.get_prev(BOTTOM_SENTINEL), BOTTOM_SENTINEL);
        assert_eq!(l.get_next(BOTTOM_SENTINEL), BOTTOM_SENTINEL);
    }

    #[test]
    #[should_panic]
    fn free_list_access_out_of_bounds() {
        let l = IntArrayFreeList::new(LIST_SIZE, 1, 1);
        l.get_size((LIST_SIZE + 1) as i32);
    }

    #[test]
    fn alloc_fit() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let result = l.alloc(2);
        assert_eq!(result, 0);

        const NEXT: i32 = 2;

        assert_eq!(l.get_prev(TOP_SENTINEL), LAST_UNIT);
        assert_eq!(l.get_next(TOP_SENTINEL), NEXT);

        assert_eq!(l.get_size(FIRST_UNIT), 2);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), -1);
        assert_eq!(l.get_right(FIRST_UNIT), 2);
        assert_eq!(l.get_next(FIRST_UNIT), 2);
        assert!(!l.is_free(FIRST_UNIT)); // not free
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(l.is_multi(FIRST_UNIT));

        assert_eq!(l.get_size(2), 2);
        assert_eq!(l.get_left(2), 0);
        assert_eq!(l.get_prev(2), -1); // no prev now
        assert_eq!(l.get_right(2), 4);
        assert_eq!(l.get_next(2), 4);
        assert!(l.is_free(2));
        assert!(l.is_coalescable(2));
        assert!(l.is_multi(2));
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // extensive checks, and it doesn't matter for tests
    fn alloc_split() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let result = l.alloc(1);
        assert_eq!(result, 0);

        const NEXT: i32 = 1;
        assert_eq!(l.get_prev(TOP_SENTINEL), LAST_UNIT);
        assert_eq!(l.get_next(TOP_SENTINEL), NEXT);

        assert_eq!(l.get_size(FIRST_UNIT), 1);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), 1); // prev is 1 now
        assert_eq!(l.get_right(FIRST_UNIT), 1); // right is 1 now
        assert_eq!(l.get_next(FIRST_UNIT), 2);
        assert!(!l.is_free(FIRST_UNIT)); // not free
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(!l.is_multi(FIRST_UNIT)); // not multi

        assert_eq!(l.get_size(1), 1);
        assert_eq!(l.get_left(1), 0); // unit1's left is 0
        assert_eq!(l.get_prev(1), -1); // unit1's prev is -1 (no prev, unit1 is removed form the list)
        assert_eq!(l.get_right(1), 2);
        assert_eq!(l.get_next(1), 2);
        assert!(l.is_free(1)); // not free
        assert!(l.is_coalescable(1));
        assert!(!l.is_multi(1)); // not multi

        assert_eq!(l.get_size(2), 2);
        assert_eq!(l.get_left(2), 1);
        assert_eq!(l.get_prev(2), 1); // uni2's prev is 1 now
        assert_eq!(l.get_right(2), 4);
        assert_eq!(l.get_next(2), 4);
        assert!(l.is_free(2));
        assert!(l.is_coalescable(2));
        assert!(l.is_multi(2));
    }

    #[test]
    fn alloc_split_twice() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        // Alloc size 1 and cause split
        let res1 = l.alloc(1);
        assert_eq!(res1, 0);
        // Alloc size 1
        let res2 = l.alloc(1);
        assert_eq!(res2, 1);

        // Next available unit has no prev now
        assert_eq!(l.get_prev(2), -1);
    }

    #[test]
    fn alloc_skip() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        // Alloc size 1 and cause split
        let res1 = l.alloc(1);
        assert_eq!(res1, 0);
        // Alloc size 2, we skip unit1
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // unit1 is still free, and linked with unit4
        assert!(l.is_free(1));
        assert_eq!(l.get_next(1), 4);
        assert_eq!(l.get_prev(4), 1);
    }

    #[test]
    fn alloc_exhaust() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);
        let res3 = l.alloc(2);
        assert_eq!(res3, FAILURE);
    }

    #[test]
    fn free_unit() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // Unit4 is still free, but has no prev
        assert_eq!(l.get_prev(4), -1);

        // Free Unit2
        let freed = l.free(res2, false);
        assert_eq!(freed, res2);
        assert!(l.is_free(res2));
    }

    #[test]
    fn free_coalesce() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // Free Unit2. It will coalesce with Unit4
        let coalesced_size = l.free(res2, true);
        assert_eq!(coalesced_size, 3);
    }

    #[test]
    fn free_cant_coalesce() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);
        let res3 = l.alloc(1);
        assert_eq!(res3, 4);

        // Free Unit2. It cannot coalesce with Unit4
        let coalesced_size = l.free(res2, true);
        assert_eq!(coalesced_size, 2);
    }

    #[test]
    fn free_realloc() {
        let mut l = IntArrayFreeList::new(LIST_SIZE, 2, 1);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // Unit4 is still free, but has no prev
        assert_eq!(l.get_prev(4), -1);

        // Free Unit2
        let freed = l.free(res2, false);
        assert_eq!(freed, res2);
        assert!(l.is_free(res2));

        // Alloc again
        let res3 = l.alloc(2);
        assert_eq!(res3, 2);
        assert!(!l.is_free(res3));

        let res4 = l.alloc(1);
        assert_eq!(res4, 4);
    }

    #[test]
    fn multi_heads_alloc_free() {
        let parent = IntArrayFreeList::new(LIST_SIZE, 1, 2);
        let mut child1 = IntArrayFreeList::from_parent(&parent, 0);
        let child2 = IntArrayFreeList::from_parent(&parent, 1);

        // child1 alloc
        let res = child1.alloc(1);
        assert_eq!(res, 0);
        assert!(!parent.is_free(0));
        assert!(!child1.is_free(0));
        assert!(!child2.is_free(0));

        // child1 free
        child1.free(0, false);
        assert!(parent.is_free(0));
        assert!(child1.is_free(0));
        assert!(child2.is_free(0));
    }

    #[test]
    #[should_panic]
    fn multi_heads_exceed_heads() {
        let parent = IntArrayFreeList::new(LIST_SIZE, 1, 2);
        let _child1 = IntArrayFreeList::from_parent(&parent, 0);
        let _child2 = IntArrayFreeList::from_parent(&parent, 1);
        let _child3 = IntArrayFreeList::from_parent(&parent, 2);
    }
}
