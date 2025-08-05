use crate::policy::compressor::region;
use crate::policy::compressor::region::CompressorRegion;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::heap::layout::VMMap;
use crate::util::heap::pageresource::{CommonPageResource, PRAllocFail, PRAllocResult};
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::Address;
use crate::util::VMThread;
use crate::util::linear_scan::Region;
use crate::util::object_enum::ObjectEnumerator;
use crate::vm::VMBinding;
use std::sync::Mutex;

pub struct RegionAllocator {
    pub region: CompressorRegion,
    pub cursor: Address,
}

pub struct Bookkeeping {
    pub all_regions: Vec<RegionAllocator>,
    pub allocation_cursor: usize,
}

pub struct CompressorPageResource<VM: VMBinding> {
    mpr: MonotonePageResource<VM>,
    pub bookkeeping: Mutex<Bookkeeping>,
}

impl<VM: VMBinding> PageResource<VM> for CompressorPageResource<VM> {
    fn common(&self) -> &CommonPageResource {
        self.mpr.common()
    }

    fn common_mut(&mut self) -> &mut CommonPageResource {
        self.mpr.common_mut()
    }

    fn update_discontiguous_start(&mut self, start: Address) {
        self.mpr.update_discontiguous_start(start)
    }

    fn alloc_pages(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        debug_assert_eq!(reserved_pages, required_pages);
        debug_assert_eq!(reserved_pages, Self::TLAB_PAGES);
        self.alloc(space_descriptor, tls)
    }

    fn get_available_physical_pages(&self) -> usize {
        self.mpr.get_available_physical_pages()
    }
}

impl<VM: VMBinding> CompressorPageResource<VM> {
    // Same as crate::util::alloc::bumpallocator::BLOCK_SIZE
    const TLAB_PAGES: usize = 8;
    const TLAB_BYTES: usize = Self::TLAB_PAGES * BYTES_IN_PAGE;
    const REGION_PAGES: usize = region::CompressorRegion::BYTES / BYTES_IN_PAGE;
    
    pub fn new_contiguous(
        start: Address,
        bytes: usize,
        vm_map: &'static dyn VMMap,
    ) -> Self {
        Self::new(MonotonePageResource::new_contiguous(start, bytes, vm_map))
    }

    pub fn new_discontiguous(
        vm_map: &'static dyn VMMap
    ) -> Self {
        Self::new(MonotonePageResource::new_discontiguous(vm_map))
    }

    fn new(mpr: MonotonePageResource<VM>) -> Self {
        Self {
            mpr,
            bookkeeping: Mutex::new(Bookkeeping {
                all_regions: vec![],
                allocation_cursor: 0,
            })
        }
    }
    
    fn alloc(
        &self,
        space_descriptor: SpaceDescriptor,
        tls: VMThread
    ) -> Result<PRAllocResult, PRAllocFail> {
        let mut b = self.bookkeeping.lock().unwrap();
        let succeed = |start: Address, new_chunk: bool| {
            Result::Ok(PRAllocResult {
                start: start,
                pages: Self::TLAB_PAGES,
                new_chunk
            })
        };
        // First try to reuse a region.
        while b.allocation_cursor < b.all_regions.len() {
            let cursor = b.allocation_cursor;
            if let Option::Some(address) =
                self.allocate_tlab(&mut b.all_regions[cursor]) {
                    self.commit_pages(Self::TLAB_PAGES, Self::TLAB_PAGES, tls);
                    self.common().accounting.commit(Self::TLAB_PAGES);
                    return succeed(address, false);
            }
            b.allocation_cursor += 1;
        }
        // Else allocate a new region.
        let PRAllocResult { start, new_chunk, .. } =
            self.mpr.alloc_pages(space_descriptor, Self::REGION_PAGES, Self::REGION_PAGES, tls)?;
        b.all_regions.push(RegionAllocator {
            region: CompressorRegion::from_aligned_address(start),
            cursor: start,
        });
        let cursor = b.allocation_cursor;
        succeed(self.allocate_tlab(&mut b.all_regions[cursor]).unwrap(), new_chunk)
    }

    pub fn allocate_tlab(&self, alloc: &mut RegionAllocator) -> Option<Address> {
        let free = alloc.cursor;
        if free >= alloc.region.end() {
            Option::None
        } else {
            alloc.cursor = free + Self::TLAB_BYTES;
            Option::Some(free)
        }
    }

    pub fn reset_cursor(&self, alloc: &mut RegionAllocator, address: Address) {
        let old = alloc.cursor;
        let new = address.align_up(Self::TLAB_BYTES);
        let pages = (old - new) / BYTES_IN_PAGE;
        self.common().accounting.release(pages);
        alloc.cursor = new;
    }

    pub fn enumerate(&self, enumerator: &mut dyn ObjectEnumerator) {
        let bookkeeping = self.bookkeeping.lock().unwrap();
        for alloc in bookkeeping.all_regions.iter() {
            enumerator.visit_address_range(alloc.region.start(), alloc.cursor);
        }
    }
}
