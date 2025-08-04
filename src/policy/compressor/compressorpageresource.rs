use crate::policy::compressor::region;
use crate::policy::compressor::region::CompressorRegion;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::heap::blockpageresource::BlockPool;
use crate::util::heap::layout::VMMap;
use crate::util::heap::pageresource::{CommonPageResource, PRAllocFail, PRAllocResult};
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::Address;
use crate::util::VMThread;
use crate::util::linear_scan::Region;
use crate::util::object_enum::ObjectEnumerator;
use crate::vm::VMBinding;
use std::sync::Mutex;

pub struct Bookkeeping {
    pub all_regions: Vec<CompressorRegion>,
    last_region: Option<CompressorRegion>,
    pub reusable_regions: BlockPool<CompressorRegion>,
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
    const TLAB_PAGES: usize = region::CompressorRegion::TLAB_BYTES >> LOG_BYTES_IN_PAGE as usize;
    const REGION_PAGES: usize = region::CompressorRegion::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    
    pub fn new_contiguous(
        start: Address,
        bytes: usize,
        vm_map: &'static dyn VMMap,
        num_workers: usize,
    ) -> Self {
        Self::new(MonotonePageResource::new_contiguous(start, bytes, vm_map), num_workers)
    }

    pub fn new_discontiguous(
        vm_map: &'static dyn VMMap,
        num_workers: usize,
    ) -> Self {
        Self::new(MonotonePageResource::new_discontiguous(vm_map), num_workers)
    }

    fn new(mpr: MonotonePageResource<VM>, num_workers: usize) -> Self {
        Self {
            mpr,
            bookkeeping: Mutex::new(Bookkeeping {
                all_regions: vec![],
                last_region: Option::None,
                reusable_regions: BlockPool::new(num_workers),
            })
        }
    }
    
    fn alloc(
        &self,
        space_descriptor: SpaceDescriptor,
        tls: VMThread
    ) -> Result<PRAllocResult, PRAllocFail> {
        let mut bookkeeping = self.bookkeeping.lock().unwrap();
        let succeed = |start: Address, new_chunk: bool| {
            Result::Ok(PRAllocResult {
                start: start,
                pages: Self::TLAB_PAGES,
                new_chunk
            })
        };
        // First try to reuse a region.
        loop {
            match bookkeeping.last_region {
                Option::Some(region) =>
                    if let Option::Some(address) = region.allocate_tlab() {
                        return succeed(address, false);
                    },
                Option::None => {
                    bookkeeping.last_region = bookkeeping.reusable_regions.pop();
                    if bookkeeping.last_region.is_none() {
                        break;
                    }
                }
            }
        }
        // Else allocate a new region.
        let PRAllocResult { start, new_chunk, .. } =
            self.mpr.alloc_pages(space_descriptor, Self::REGION_PAGES, Self::REGION_PAGES, tls)?;
        let region = CompressorRegion::from_aligned_address(start);
        region.initialise();
        bookkeeping.all_regions.push(region);
        if let Option::Some(address) = region.allocate_tlab() {
            succeed(address, new_chunk)
        } else {
            Result::Err(PRAllocFail)
        }
    }

    pub fn enumerate(&self, enumerator: &mut dyn ObjectEnumerator) {
        let bookkeeping = self.bookkeeping.lock().unwrap();
        for r in bookkeeping.all_regions.iter() {
            enumerator.visit_address_range(r.start(), r.end());
        }
    }
}
