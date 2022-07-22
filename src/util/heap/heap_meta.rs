use crate::util::Address;
use crate::util::options::Options;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::conversions;

pub struct HeapMeta {
    pub heap_cursor: Address,
    pub heap_limit: Address,
    pub total_pages: usize,
}

impl HeapMeta {
    pub fn new(options: &Options) -> Self {
        HeapMeta {
            heap_cursor: HEAP_START,
            heap_limit: HEAP_END,
            total_pages: conversions::bytes_to_pages(*options.heap_size),
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

    pub fn get_total_pages(&self) -> usize {
        self.total_pages
    }
}
