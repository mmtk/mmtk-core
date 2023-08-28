use downcast_rs::{impl_downcast, Downcast};

use super::Address;

pub const FAILURE: i32 = -1;

pub const MAX_HEADS: i32 = 128; // somewhat arbitrary
const TOTAL_BITS: i32 = 32;
const UNIT_BITS: i32 = TOTAL_BITS - 2;
pub const MAX_UNITS: i32 = ((1 << UNIT_BITS) - 1) - MAX_HEADS - 1;

const NEXT_MASK: i32 = (1 << UNIT_BITS) - 1;
const PREV_MASK: i32 = (1 << UNIT_BITS) - 1;
const FREE_MASK: i32 = 1 << (TOTAL_BITS - 1);
const MULTI_MASK: i32 = 1 << (TOTAL_BITS - 1);
const COALESC_MASK: i32 = 1 << (TOTAL_BITS - 2);
const SIZE_MASK: i32 = (1 << UNIT_BITS) - 1;

// TODO: FreeList should not implement Sync.
// FreeList instances are not thread-safe.
// They need external synchronisation (e.g. using Mutex).
// On the other hand, to put FreeList into a Mutex<T>, FreeList must implement Send.
// There is no problem sending FreeList instances between threads.
pub trait FreeList: Sync + Send + Downcast {
    fn head(&self) -> i32;
    // fn head_mut(&mut self) -> &mut i32;
    fn heads(&self) -> i32;
    // fn heads_mut(&mut self) -> &mut i32;
    // fn resize_freelist(&mut self);
    // fn resize_freelist(&mut self, units: i32, heads: i32);
    fn get_entry(&self, index: i32) -> i32;
    fn set_entry(&mut self, index: i32, value: i32);

    // Workaround space start calculation.
    // TODO: This is a hack, and is unlikely to be the final solution.
    fn maybe_get_limit(&self) -> Option<Address>;

    fn alloc(&mut self, size: i32) -> i32 {
        let mut unit = self.head();
        let mut s = 0;
        while ({
            unit = self.get_next(unit);
            unit != self.head()
        }) && ({
            s = self.get_size(unit);
            s < size
        }) {}
        // loop {
        //   unit = self.get_next(unit);
        //   // println!("Current unit={}", unit);
        //   if unit != self.head() {
        //     break;
        //   }
        //   s = self.get_size(unit);
        //   if s < size {
        //     break;
        //   }
        // }
        if unit == self.head() {
            FAILURE
        } else {
            self.__alloc(size, unit, s)
        }
    }

    fn alloc_from_unit(&mut self, size: i32, unit: i32) -> i32 {
        if self.get_free(unit) {
            let s = self.get_size(unit);
            if s >= size {
                return self.__alloc(size, unit, s);
            }
        }
        FAILURE
    }

    /// Free a previously allocated contiguous lump of units
    fn free(&mut self, unit: i32, return_coalesced_size: bool) -> i32 {
        debug_assert!(!self.get_free(unit));
        let mut freed = self.get_size(unit);
        let left = self.get_left(unit);
        let start = if self.is_coalescable(unit) && self.get_free(left) {
            left
        } else {
            unit
        };
        let right = self.get_right(unit);
        let end = if self.is_coalescable(right) && self.get_free(right) {
            right
        } else {
            unit
        };
        if start != end {
            self.__coalesce(start, end);
        }

        if return_coalesced_size {
            freed = self.get_size(start);
        }
        self.add_to_free(start);

        freed
    }

    fn size(&self, unit: i32) -> i32 {
        self.get_size(unit)
    }

    fn initialize_heap(&mut self, units: i32, grain: i32) {
        // Initialize the sentinels
        // Set top sentinels per heads
        for i in 1..=self.heads() {
            self.set_sentinel(-i);
        }
        // Set bottom sentinel
        self.set_sentinel(units);

        // create the free list item
        let offset = units % grain;
        let mut cursor = units - offset;
        if offset > 0 {
            self.set_size(cursor, offset);
            self.add_to_free(cursor);
        }
        cursor -= grain;
        while cursor >= 0 {
            self.set_size(cursor, grain);
            self.add_to_free(cursor);
            cursor -= grain;
        }
    }

    fn add_to_free(&mut self, unit: i32) {
        self.set_free(unit, true);
        let next = self.get_next(self.head());
        self.set_next(unit, next);
        let head = self.head();
        self.set_next(head, unit);
        let head = self.head();
        self.set_prev(unit, head);
        self.set_prev(next, unit);
    }

    fn get_right(&self, unit: i32) -> i32 {
        unit + self.get_size(unit)
    }

    fn set_sentinel(&mut self, unit: i32) {
        self.set_lo_entry(unit, NEXT_MASK & unit);
        self.set_hi_entry(unit, PREV_MASK & unit);
    }

    fn get_size(&self, unit: i32) -> i32 {
        if (self.get_hi_entry(unit) & MULTI_MASK) == MULTI_MASK {
            self.get_hi_entry(unit + 1) & SIZE_MASK
        } else {
            1
        }
    }

    fn set_size(&mut self, unit: i32, size: i32) {
        let hi = self.get_hi_entry(unit);
        if size > 1 {
            self.set_hi_entry(unit, hi | MULTI_MASK);
            self.set_hi_entry(unit + 1, MULTI_MASK | size);
            self.set_hi_entry(unit + size - 1, MULTI_MASK | size);
        } else {
            self.set_hi_entry(unit, hi & !MULTI_MASK);
        }
    }

    fn get_free(&self, unit: i32) -> bool {
        (self.get_lo_entry(unit) & FREE_MASK) == FREE_MASK
    }

    fn set_free(&mut self, unit: i32, is_free: bool) {
        let size;
        let lo = self.get_lo_entry(unit);
        if is_free {
            self.set_lo_entry(unit, lo | FREE_MASK);
            size = self.get_size(unit);
            if size > 1 {
                let lo = self.get_lo_entry(unit + size - 1);
                self.set_lo_entry(unit + size - 1, lo | FREE_MASK);
            }
        } else {
            self.set_lo_entry(unit, lo & !FREE_MASK);
            size = self.get_size(unit);
            if size > 1 {
                let lo = self.get_lo_entry(unit + size - 1);
                self.set_lo_entry(unit + size - 1, lo & !FREE_MASK);
            }
        }
    }

    fn get_next(&self, unit: i32) -> i32 {
        let next = self.get_hi_entry(unit) & NEXT_MASK;
        if next <= MAX_UNITS {
            next
        } else {
            self.head()
        }
    }

    fn set_next(&mut self, unit: i32, next: i32) {
        debug_assert!((next >= -self.heads()) && (next <= MAX_UNITS));
        let old_value = self.get_hi_entry(unit);
        let new_value = (old_value & !NEXT_MASK) | (next & NEXT_MASK);
        self.set_hi_entry(unit, new_value);
    }

    // Return the previous link. If no previous link, return head
    fn get_prev(&self, unit: i32) -> i32 {
        let prev = self.get_lo_entry(unit) & PREV_MASK;
        if prev <= MAX_UNITS {
            prev
        } else {
            self.head()
        }
    }

    fn set_prev(&mut self, unit: i32, prev: i32) {
        debug_assert!((prev >= -self.heads()) && (prev <= MAX_UNITS));
        let old_value = self.get_lo_entry(unit);
        let new_value = (old_value & !PREV_MASK) | (prev & PREV_MASK);
        self.set_lo_entry(unit, new_value);
    }

    // Return the left unit. If it is a multi unit, return the start of the unit.
    fn get_left(&self, unit: i32) -> i32 {
        if (self.get_hi_entry(unit - 1) & MULTI_MASK) == MULTI_MASK {
            unit - (self.get_hi_entry(unit - 1) & SIZE_MASK)
        } else {
            unit - 1
        }
    }

    fn is_coalescable(&self, unit: i32) -> bool {
        (self.get_lo_entry(unit) & COALESC_MASK) == 0
    }

    fn clear_uncoalescable(&mut self, unit: i32) {
        let lo = self.get_lo_entry(unit);
        self.set_lo_entry(unit, lo & !COALESC_MASK);
    }

    fn set_uncoalescable(&mut self, unit: i32) {
        let lo = self.get_lo_entry(unit);
        self.set_lo_entry(unit, lo | COALESC_MASK);
    }

    fn is_multi(&self, i: i32) -> bool {
        let hi = self.get_hi_entry(i);
        (hi & MULTI_MASK) == MULTI_MASK
    }

    fn is_free(&self, i: i32) -> bool {
        let lo = self.get_lo_entry(i);
        (lo & FREE_MASK) == FREE_MASK
    }

    fn get_lo_entry(&self, unit: i32) -> i32 {
        self.get_entry((unit + self.heads()) << 1)
    }

    fn get_hi_entry(&self, unit: i32) -> i32 {
        self.get_entry(((unit + self.heads()) << 1) + 1)
    }

    fn set_lo_entry(&mut self, unit: i32, value: i32) {
        let heads = self.heads();
        self.set_entry((unit + heads) << 1, value);
    }

    fn set_hi_entry(&mut self, unit: i32, value: i32) {
        let heads = self.heads();
        self.set_entry(((unit + heads) << 1) + 1, value);
    }

    // Private methods

    fn __alloc(&mut self, size: i32, unit: i32, unit_size: i32) -> i32 {
        if unit_size >= size {
            if unit_size > size {
                self.__split(unit, size);
            }
            self.__remove_from_free(unit);
            self.set_free(unit, false);
        }
        unit
    }

    fn __split(&mut self, unit: i32, size: i32) {
        let basesize = self.get_size(unit);
        debug_assert!(basesize > size);
        self.set_size(unit, size);
        self.set_size(unit + size, basesize - size);
        self.add_to_free(unit + size);
    }

    fn __coalesce(&mut self, start: i32, end: i32) {
        if self.get_free(end) {
            self.__remove_from_free(end);
        }
        if self.get_free(start) {
            self.__remove_from_free(start);
        }
        let size = self.get_size(end);
        self.set_size(start, end - start + size);
    }

    fn __remove_from_free(&mut self, unit: i32) {
        let next = self.get_next(unit);
        let prev = self.get_prev(unit);
        self.set_next(prev, next);
        self.set_prev(next, prev);
    }
}

impl_downcast!(FreeList);
