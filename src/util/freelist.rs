use downcast_rs::{impl_downcast, Downcast};

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

/// This is a very simple, generic malloc-free allocator.  It works abstractly, in "units", which
/// the user may associate with some other allocatable resource (e.g. heap blocks).  The user
/// issues requests for N units and the allocator returns the index of the first of a contiguous
/// set of N units or fails, returning -1.  The user frees the block of N units by calling `free()`
/// with the index of the first unit as the argument.
///
/// Properties/Constraints:
///
/// -   The allocator consumes one word per allocatable unit (plus a fixed overhead of about 128
///     words).
/// -   The allocator can only deal with `MAX_UNITS` units (see below for the value).
///
/// The basic data structure used by the algorithm is a large table, with one word per allocatable
/// unit.  Each word is used in a number of different ways, some combination of "undefined" (32),
/// "free/used" (1), "multi/single" (1), "prev" (15), "next" (15) and "size" (15) where field sizes
/// in bits are in parenthesis.
///
/// ```
///                       +-+-+-----------+-----------+
///                       |f|m|    prev   | next/size |
///                       +-+-+-----------+-----------+
/// ```
///
/// -   single free unit: "free", "single", "prev", "next"
/// -   single used unit: "used", "single"
/// -   contiguous free units
///     *   first unit: "free", "multi", "prev", "next"
///     *   second unit: "free", "multi", "size"
///     *   last unit: "free", "multi", "size"
/// -   contiguous used units
///     *   first unit: "used", "multi", "prev", "next"
///     *   second unit: "used", "multi", "size"
///     *   last unit: "used", "multi", "size"
/// -   any other unit: undefined
///
/// ```
///                       +-+-+-----------+-----------+
///   top sentinel        |0|0|    tail   |   head    |  [-1]
///                       +-+-+-----------+-----------+
///                                     ....
///            /--------  +-+-+-----------+-----------+
///            |          |1|1|   prev    |   next    |  [j]
///            |          +-+-+-----------+-----------+
///            |          |1|1|           |   size    |  [j+1]
///         free multi    +-+-+-----------+-----------+
///         unit block    |              ...          |  ...
///            |          +-+-+-----------+-----------+
///            |          |1|1|           |   size    |
///            >--------  +-+-+-----------+-----------+
///   single free unit    |1|0|   prev    |   next    |
///            >--------  +-+-+-----------+-----------+
///   single used unit    |0|0|                       |
///            >--------  +-+-+-----------------------+
///            |          |0|1|                       |
///            |          +-+-+-----------+-----------+
///            |          |0|1|           |   size    |
///         used multi    +-+-+-----------+-----------+
///         unit block    |              ...          |
///            |          +-+-+-----------+-----------+
///            |          |0|1|           |   size    |
///            \--------  +-+-+-----------+-----------+
///                                     ....
///                       +-+-+-----------------------+
///   bottom sentinel     |0|0|                       |  [N]
///                       +-+-+-----------------------+
/// ```
///
/// The sentinels serve as guards against out of range coalescing because they both appear as
/// "used" blocks and so will never coalesce.  The top sentinel also serves as the head and tail of
/// the doubly linked list of free blocks.
pub trait FreeList: Sync + Downcast {
    fn head(&self) -> i32;
    // fn head_mut(&mut self) -> &mut i32;

    /// The number of free lists which will share this instance
    fn heads(&self) -> i32;

    // fn heads_mut(&mut self) -> &mut i32;
    // fn resize_freelist(&mut self);
    // fn resize_freelist(&mut self, units: i32, heads: i32);

    /// Fetch the value at the given index into the table.
    ///
    /// # Parameters
    ///
    /// -   `index`: Index of the value to fetch.  Note this is a table index, not a unit number.
    ///
    /// # Returns
    ///
    /// Contents of the given index.
    fn get_entry(&self, index: i32) -> i32;

    /// Store the given value at an index into the table
    ///
    /// # Parameters
    ///
    /// -   `index`: Index of the entry to fetch.  Note this is a table index, not a unit number.
    /// -   `value`: The value to store.
    fn set_entry(&mut self, index: i32, value: i32);

    /// Allocate `size` units. Return the unit ID
    ///
    /// # Parameters
    ///
    /// -   `size`: The number of units to be allocated
    ///
    /// # Returns
    ///
    /// The index of the first of the `size` contiguous units, or -1 if the request can't be
    /// satisfied.
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
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit.
    /// -   `return_coalesced_size`: If true, return the coalesced size.
    ///
    /// # Returns
    ///
    /// Return the size of the unit which was freed. If `return_coalesced_size` is false, return
    /// the size of the unit which was freed.  Otherwise return the size of the unit now available
    /// (the coalesced size).
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

    /// Return the size of the specified lump of units.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the lump.
    ///
    /// # Returns
    ///
    /// The size of the lump, in units.
    fn size(&self, unit: i32) -> i32 {
        self.get_size(unit)
    }

    /// Initialize a new heap.  Fabricate a free list entry containing everything.
    ///
    /// # Parameters
    ///
    /// -   `units`: The number of units in the heap.
    /// -   `grain`: TODO needs documentation
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

    /// Add a lump of units to the free list
    ///
    /// # Parameters
    ///
    /// -   `unit`: The first unit in the lump of units to be added.
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

    /// Get the lump to the "right" of the current lump (i.e. "below" it).
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the lump in question.
    ///
    /// # Returns
    ///
    /// The index of the first unit in the lump to the "right"/"below" the lump in question.
    ///
    /// # Known issues
    ///
    /// People using right-to-left or top-to-bottom languages (including those modern Chinese and
    /// Japanese who still prefer writing vertically) may be confused about the notion of "right";
    /// and system programmers who are familiar with "low address" may be confused about the notion
    /// of "below".  Consider renaming to `next_adjacent_lump`.
    fn get_right(&self, unit: i32) -> i32 {
        unit + self.get_size(unit)
    }

    /// Initialize a unit as a sentinel.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The unit to be initialized
    fn set_sentinel(&mut self, unit: i32) {
        self.set_lo_entry(unit, NEXT_MASK & unit);
        self.set_hi_entry(unit, PREV_MASK & unit);
    }

    /// Get the size of a lump of units
    ///
    /// # Parameters
    ///
    /// -   `unit`: The first unit in the lump of units
    ///
    /// # Returns
    ///
    /// The size of the lump of units
    fn get_size(&self, unit: i32) -> i32 {
        if (self.get_hi_entry(unit) & MULTI_MASK) == MULTI_MASK {
            self.get_hi_entry(unit + 1) & SIZE_MASK
        } else {
            1
        }
    }

    /// Set the size of lump of units.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The first unit in the lump of units.
    /// -   `size`: The size of the lump of units.
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

    /// Establish whether a lump of units is free.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The first or last unit in the lump.
    ///
    /// # Returns
    ///
    /// `true` if the lump is free.
    fn get_free(&self, unit: i32) -> bool {
        (self.get_lo_entry(unit) & FREE_MASK) == FREE_MASK
    }

    /// Set the "free" flag for a lump of units (both the first and last units in the lump are set.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The first unit in the lump.
    /// -   `is_free`: `true` if the lump is to be marked as free.
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

    /// Get the next lump in the doubly linked free list.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the current lump.
    ///
    /// # Returns
    ///
    /// The index of the first unit of the next lump of units in the list.
    fn get_next(&self, unit: i32) -> i32 {
        let next = self.get_hi_entry(unit) & NEXT_MASK;
        if next <= MAX_UNITS {
            next
        } else {
            self.head()
        }
    }

    /// Set the next lump in the doubly linked free list.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the lump to be set.
    /// -   `next`: The value to be set.
    fn set_next(&mut self, unit: i32, next: i32) {
        debug_assert!((next >= -self.heads()) && (next <= MAX_UNITS));
        let old_value = self.get_hi_entry(unit);
        let new_value = (old_value & !NEXT_MASK) | (next & NEXT_MASK);
        self.set_hi_entry(unit, new_value);
    }

    /// Get the previous lump in the doubly linked free list.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the current lump.
    ///
    /// # Returns
    ///
    /// The index of the first unit of the previous lump of units in the list.  If there is no
    /// previous link, return the head.
    fn get_prev(&self, unit: i32) -> i32 {
        let prev = self.get_lo_entry(unit) & PREV_MASK;
        if prev <= MAX_UNITS {
            prev
        } else {
            self.head()
        }
    }

    /// Set the previous lump in the doubly linked free list.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the lump to be set.
    /// -   `prev`: The value to be set.
    fn set_prev(&mut self, unit: i32, prev: i32) {
        debug_assert!((prev >= -self.heads()) && (prev <= MAX_UNITS));
        let old_value = self.get_lo_entry(unit);
        let new_value = (old_value & !PREV_MASK) | (prev & PREV_MASK);
        self.set_lo_entry(unit, new_value);
    }

    /// Get the lump to the "left" of the current lump (i.e. "above" it)
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit in the lump in question
    ///
    /// # Returns
    ///
    /// The index of the first unit in the lump to the "left"/"above" the lump in question.
    ///
    /// # Known Issues
    ///
    /// Similar to `get_right`, we should rename it to `previous_adjacent_lump`.
    fn get_left(&self, unit: i32) -> i32 {
        if (self.get_hi_entry(unit - 1) & MULTI_MASK) == MULTI_MASK {
            unit - (self.get_hi_entry(unit - 1) & SIZE_MASK)
        } else {
            unit - 1
        }
    }

    /// Return true if this unit may be coalesced with the unit below it.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The unit in question
    ///
    /// # Returns
    ///
    /// `true` if this unit may be coalesced with the unit below it.
    ///
    /// # Known issue
    ///
    /// Similar to `get_right` we should avoid the notion of "left", "right", "above" or "below".
    /// The "next adjacent unit" should be fine to explain this function.
    fn is_coalescable(&self, unit: i32) -> bool {
        (self.get_lo_entry(unit) & COALESC_MASK) == 0
    }

    /// Clear the Uncoalescable flag associated with a unit.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The unit in question
    fn clear_uncoalescable(&mut self, unit: i32) {
        let lo = self.get_lo_entry(unit);
        self.set_lo_entry(unit, lo & !COALESC_MASK);
    }

    /// Mark a unit as uncoalescable
    ///
    /// # Parameters
    ///
    /// -   `unit`: The unit in question
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

    /// Get the (low) contents of an entry.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the unit.
    ///
    /// # Returns
    ///
    /// The (low) contents of the unit.
    fn get_lo_entry(&self, unit: i32) -> i32 {
        self.get_entry((unit + self.heads()) << 1)
    }

    /// Get the (high) contents of an entry
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the unit.
    ///
    /// # Returns
    ///
    /// The (high) contents of the unit.
    fn get_hi_entry(&self, unit: i32) -> i32 {
        self.get_entry(((unit + self.heads()) << 1) + 1)
    }

    /// Set the (low) contents of an entry
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the unit
    /// -   `value`: The (low) contents of the unit
    fn set_lo_entry(&mut self, unit: i32, value: i32) {
        let heads = self.heads();
        self.set_entry((unit + heads) << 1, value);
    }

    /// Set the (hi) contents of an entry
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the unit
    /// -   `value`: The (hi) contents of the unit
    fn set_hi_entry(&mut self, unit: i32, value: i32) {
        let heads = self.heads();
        self.set_entry(((unit + heads) << 1) + 1, value);
    }

    // Private methods

    /// Allocate `size` units. Return the unit ID
    ///
    /// # Parameters
    ///
    /// -   `size`: The number of units to be allocated
    /// -   `unit`: First unit to consider
    /// -   `unit_size`: The size of the lump of units starting at `unit`
    ///
    /// # Returns
    ///
    /// The index of the first of the `size` contiguous units, or -1 if the request can't be
    /// satisfied
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

    /// Reduce a lump of units to size, freeing any excess.
    ///
    /// # Parameters
    ///
    /// -   `unit`: The index of the first unit.
    /// -   `size`: The size of the first part.
    fn __split(&mut self, unit: i32, size: i32) {
        let basesize = self.get_size(unit);
        debug_assert!(basesize > size);
        self.set_size(unit, size);
        self.set_size(unit + size, basesize - size);
        self.add_to_free(unit + size);
    }

    /// Coalesce two or three contiguous lumps of units, removing start and end lumps from the free
    /// list as necessary.
    ///
    /// # Parameters
    ///
    /// -   `start`: The index of the start of the first lump
    /// -   `end`: The index of the start of the last lump
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

    /// Remove a lump of units from the free list.
    ///
    /// # Parameters
    ///
    /// -   `unit` The first unit in the lump of units to be removed
    fn __remove_from_free(&mut self, unit: i32) {
        let next = self.get_next(unit);
        let prev = self.get_prev(unit);
        self.set_next(prev, next);
        self.set_prev(next, prev);
    }
}

impl_downcast!(FreeList);
