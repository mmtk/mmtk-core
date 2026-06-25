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

    pub fn reserve_quarantined(
        &mut self,
        extent: usize,
        align: Option<usize>,
        top: bool,
        mmapper: &dyn Mmapper,
        huge_page_option: HugePageSupport,
        anno: &MmapAnnotation,
    ) -> MmapResult<Address> {
        let start = if top {
            let raw_start = self.heap_limit - extent;
            if let Some(align) = align {
                raw_start.align_down(align)
            } else {
                raw_start
            }
        } else {
            let raw_start = self.heap_cursor;
            if let Some(align) = align {
                raw_start.align_up(align)
            } else {
                raw_start
            }
        };

        // TODO: The following call do an fixed mmap. We should try to allow the OS to choose the address if the fixed mmap fails.
        mmapper.quarantine_address_range(
            start,
            crate::util::conversions::bytes_to_pages_up(extent),
            huge_page_option,
            anno,
        )?;

        if top {
            self.heap_limit = start;
        } else {
            self.heap_cursor = start + extent;
        }

        assert!(
            self.heap_cursor <= self.heap_limit,
            "Out of virtual address space after quarantining [{}, {})",
            start,
            start + extent,
        );

        Ok(start)
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
