use crate::util::heap::layout::vm_layout::vm_layout;
use crate::util::heap::layout::Mmapper;
use crate::util::os::{HugePageSupport, MmapAnnotation, MmapResult};
use crate::util::Address;

pub struct HeapMeta {
    pub heap_cursor: Address,
    pub heap_limit: Address,
}

impl HeapMeta {
    pub fn new() -> Self {
        HeapMeta {
            heap_cursor: vm_layout().heap_start,
            heap_limit: vm_layout().heap_end,
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

    pub fn reserve_quarantined(
        &mut self,
        extent: usize,
        top: bool,
        mmapper: &dyn Mmapper,
        huge_page_option: HugePageSupport,
        anno: &MmapAnnotation,
    ) -> MmapResult<Address> {
        let preferred = if top {
            self.heap_limit - extent
        } else {
            self.heap_cursor
        };

        let actual = mmapper.quarantine_address_range_preferred(
            preferred,
            crate::util::conversions::bytes_to_pages_up(extent),
            huge_page_option,
            anno,
        )?;

        assert!(
            actual >= self.heap_cursor && actual + extent <= self.heap_limit,
            "Quarantined heap range [{}, {}) is outside available heap range [{}, {})",
            actual,
            actual + extent,
            self.heap_cursor,
            self.heap_limit,
        );

        if top {
            self.heap_limit = actual;
        } else {
            self.heap_cursor = actual + extent;
        }

        assert!(
            self.heap_cursor <= self.heap_limit,
            "Out of virtual address space after quarantining [{}, {})",
            actual,
            actual + extent,
        );

        Ok(actual)
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
