use util::Address;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

pub struct HeapMeta {
    pub heap_cursor: Address,
    pub heap_limit: Address,
    pub total_pages: AtomicUsize,
}

impl HeapMeta {
    pub fn new(start: Address, end: Address) -> Self {
        HeapMeta {
            heap_cursor: start,
            heap_limit: end,
            total_pages: AtomicUsize::new(0)
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

        if self.heap_cursor > self.heap_limit {
            panic!("Out of virtual address space at {} ({} > {})",
                   self.heap_cursor - extent, self.heap_cursor, self.heap_limit);
        }

        ret
    }

    pub fn get_discontig_start(&self) -> Address {
        self.heap_cursor
    }

    pub fn get_discontig_end(&self) -> Address {
        self.heap_limit - 1
    }

    pub fn get_total_pages(&self) -> usize {
        self.total_pages.load(Ordering::Relaxed)
    }
}
