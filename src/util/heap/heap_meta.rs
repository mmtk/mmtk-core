use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::Address;

pub struct HeapMeta {
    pub heap_cursor: Address,
    pub heap_limit: Address,
}

impl HeapMeta {
    pub fn new() -> Self {
        HeapMeta {
            heap_cursor: HEAP_START,
            heap_limit: HEAP_END,
        }
    }

    pub fn reserve(&mut self, extent: usize, top: bool) -> Address {
        let ret = if top {
            self.heap_limit -= extent;
            self.heap_limit
        } else {
            let start = self.heap_cursor;
            self.heap_cursor += extent;
            start
        };

        assert!(
            self.heap_cursor <= self.heap_limit,
            "Out of virtual address space at {} ({} > {})",
            self.heap_cursor - extent,
            self.heap_cursor,
            self.heap_limit
        );

        ret
    }

    pub fn get_discontig_start(&self) -> Address {
        self.heap_cursor
    }

    pub fn get_discontig_end(&self) -> Address {
        self.heap_limit - 1
    }
}

// make clippy happy
impl Default for HeapMeta {
    fn default() -> Self {
        Self::new()
    }
}
