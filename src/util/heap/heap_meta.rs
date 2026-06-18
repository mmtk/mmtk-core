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
        debug!(
            "Request to reserve quarantined memory for {} bytes, align {:?}, top={}",
            extent, align, top
        );
        let candidate = if top {
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
        debug!(
            "Preferred address for quarantine reservation is {}",
            candidate
        );
        let actual = if vm_layout().dynamic_heap_range {
            mmapper.quarantine_address_range_preferred(
                candidate,
                crate::util::conversions::bytes_to_pages_up(extent),
                align,
                huge_page_option,
                anno,
            )?
        } else {
            mmapper.quarantine_address_range(
                candidate,
                crate::util::conversions::bytes_to_pages_up(extent),
                huge_page_option,
                anno,
            )?;
            candidate
        };

        assert!(
            actual >= self.heap_cursor && actual + extent <= self.heap_limit,
            "Quarantined heap range [{}, {}) is outside available heap range [{}, {})",
            actual,
            actual + extent,
            self.heap_cursor,
            self.heap_limit,
        );

        if actual == candidate {
            if top {
                self.heap_limit = actual;
            } else {
                self.heap_cursor = actual + extent;
            }
        }

        assert!(
            self.heap_cursor <= self.heap_limit,
            "Out of virtual address space after quarantining [{}, {})",
            actual,
            actual + extent,
        );

        debug!(
            "Reserved quarantined memory [{}, {}) for {} bytes, align {:?}",
            actual,
            actual + extent,
            extent,
            align
        );
        debug!(
            "Available heap range after reservation is [{}, {})",
            self.heap_cursor, self.heap_limit
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
