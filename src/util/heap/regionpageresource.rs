use crate::util::constants::BYTES_IN_PAGE;
use crate::util::heap::layout::VMMap;
use crate::util::heap::pageresource::{CommonPageResource, PRAllocFail, PRAllocResult};
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::heap::{MonotonePageResource, PageResource};
use crate::util::linear_scan::Region;
use crate::util::object_enum::ObjectEnumerator;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

/// A region in a RegionPageResource and its allocation cursor.
pub struct RegionAllocator<R: Region> {
    pub region: R,
    cursor: AtomicUsize,
}

impl<R: Region> RegionAllocator<R> {
    pub fn cursor(&self) -> Address {
        let a = self.cursor.load(Ordering::Relaxed);
        unsafe { Address::from_usize(a) }
    }

    fn set_cursor(&self, a: Address) {
        self.cursor.store(a.as_usize(), Ordering::Relaxed);
    }
}

struct Sync<R: Region> {
    all_regions: Vec<RegionAllocator<R>>,
    next_region: usize,
}

/// A PageResource which allocates pages from a region-structured heap.
/// We assume that allocations are much smaller than regions, as we
/// scan linearly over all regions to allocate, and do not revisit regions
/// before a garbage collection cycle.
pub struct RegionPageResource<VM: VMBinding, R: Region> {
    mpr: MonotonePageResource<VM>,
    sync: RwLock<Sync<R>>,
}

impl<VM: VMBinding, R: Region + 'static> PageResource<VM> for RegionPageResource<VM, R> {
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
        assert!(reserved_pages <= Self::REGION_PAGES);
        assert!(required_pages <= reserved_pages);
        self.alloc(space_descriptor, reserved_pages, required_pages, tls)
    }

    fn get_available_physical_pages(&self) -> usize {
        self.mpr.get_available_physical_pages()
    }
}

impl<VM: VMBinding, R: Region + 'static> RegionPageResource<VM, R> {
    // Same as crate::util::alloc::bumpallocator::BLOCK_SIZE
    const TLAB_PAGES: usize = 8;
    const TLAB_BYTES: usize = Self::TLAB_PAGES * BYTES_IN_PAGE;
    const REGION_PAGES: usize = R::BYTES / BYTES_IN_PAGE;

    pub fn new_contiguous(start: Address, bytes: usize, vm_map: &'static dyn VMMap) -> Self {
        Self::new(MonotonePageResource::new_contiguous(start, bytes, vm_map))
    }

    pub fn new_discontiguous(vm_map: &'static dyn VMMap) -> Self {
        Self::new(MonotonePageResource::new_discontiguous(vm_map))
    }

    fn new(mpr: MonotonePageResource<VM>) -> Self {
        Self {
            mpr,
            sync: RwLock::new(Sync {
                all_regions: vec![],
                next_region: 0,
            }),
        }
    }

    fn alloc(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        let mut b = self.sync.write().unwrap();
        let succeed = |start: Address, new_chunk: bool| {
            Result::Ok(PRAllocResult {
                start,
                pages: Self::TLAB_PAGES,
                new_chunk,
            })
        };
        let bytes = reserved_pages * BYTES_IN_PAGE;
        // First try to reuse a region.
        while b.next_region < b.all_regions.len() {
            let cursor = b.next_region;
            if let Option::Some(address) =
                self.allocate_from_region(&mut b.all_regions[cursor], bytes)
            {
                self.commit_pages(reserved_pages, required_pages, tls);
                return succeed(address, false);
            }
            b.next_region += 1;
        }
        // Else allocate a new region.
        let PRAllocResult {
            start, new_chunk, ..
        } = self.mpr.alloc_pages(
            space_descriptor,
            Self::REGION_PAGES,
            Self::REGION_PAGES,
            tls,
        )?;
        b.all_regions.push(RegionAllocator {
            region: R::from_aligned_address(start),
            cursor: AtomicUsize::new(start.as_usize()),
        });
        let cursor = b.next_region;
        succeed(
            self.allocate_from_region(&mut b.all_regions[cursor], bytes)
                .unwrap(),
            new_chunk,
        )
    }

    fn allocate_from_region(
        &self,
        alloc: &mut RegionAllocator<R>,
        bytes: usize,
    ) -> Option<Address> {
        let free = alloc.cursor();
        if free + bytes > alloc.region.end() {
            Option::None
        } else {
            alloc.set_cursor(free + bytes);
            Option::Some(free)
        }
    }

    /// Reset the allocation cursor for one region.
    pub fn reset_cursor(&self, alloc: &RegionAllocator<R>, address: Address) {
        let old = alloc.cursor();
        let new = address.align_up(BYTES_IN_PAGE);
        let pages = (old - new) / BYTES_IN_PAGE;
        self.common().accounting.release(pages);
        alloc.set_cursor(new);
    }

    /// Reset the allocator state after a collection, so that the allocator will
    /// revisit regions which the garbage collector has compacted.
    pub fn reset_allocator(&self) {
        self.sync.write().unwrap().next_region = 0;
    }

    pub fn enumerate(&self, enumerator: &mut dyn ObjectEnumerator) {
        let sync = self.sync.read().unwrap();
        for alloc in sync.all_regions.iter() {
            enumerator.visit_address_range(alloc.region.start(), alloc.cursor());
        }
    }

    pub fn with_regions<T>(&self, f: &mut impl FnMut(&Vec<RegionAllocator<R>>) -> T) -> T {
        let sync = self.sync.read().unwrap();
        f(&sync.all_regions)
    }

    pub fn enumerate_regions(&self, enumerator: &mut impl FnMut(&RegionAllocator<R>)) {
        let sync = self.sync.read().unwrap();
        for alloc in sync.all_regions.iter() {
            enumerator(alloc);
        }
    }
}
