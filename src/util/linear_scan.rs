use crate::util::alloc_bit;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::marker::PhantomData;

/// Iterate over an address range, and find each object by alloc bit.
/// ATOMIC_LOAD_ALLOC_BIT can be set to false if it is known that loading alloc bit
/// non-atomically is correct (e.g. a single thread is scanning this address range, and
/// it is the only thread that accesses alloc bit).
pub struct ObjectIterator<VM: VMBinding, S: LinearScanObjectSize, const ATOMIC_LOAD_ALLOC_BIT: bool>
{
    start: Address,
    end: Address,
    cursor: Address,
    _p: PhantomData<(VM, S)>,
}

impl<VM: VMBinding, S: LinearScanObjectSize, const ATOMIC_LOAD_ALLOC_BIT: bool>
    ObjectIterator<VM, S, ATOMIC_LOAD_ALLOC_BIT>
{
    /// Create an iterator for the address range. The caller must ensure
    /// that the alloc bit metadata is mapped for the address range.
    pub fn new(start: Address, end: Address) -> Self {
        debug_assert!(start < end);
        ObjectIterator {
            start,
            end,
            cursor: start,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, S: LinearScanObjectSize, const ATOMIC_LOAD_ALLOC_BIT: bool> std::iter::Iterator
    for ObjectIterator<VM, S, ATOMIC_LOAD_ALLOC_BIT>
{
    type Item = ObjectReference;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        while self.cursor < self.end {
            let is_object = if ATOMIC_LOAD_ALLOC_BIT {
                alloc_bit::is_alloced_object::<VM>(self.cursor)
            } else {
                unsafe { alloc_bit::is_alloced_object_unsafe::<VM>(self.cursor) }
            };

            if let Some(object) = is_object {
                self.cursor += S::size(object);
                return Some(object);
            } else {
                self.cursor += VM::MIN_ALIGNMENT;
            }
        }

        None
    }
}

/// Describe object size for linear scan. Different policies may have
/// different object sizes (e.g. extra metadata, etc)
pub trait LinearScanObjectSize {
    fn size(object: ObjectReference) -> usize;
}

/// Default object size as ObjectModel::get_current_size()
pub struct DefaultObjectSize<VM: VMBinding>(PhantomData<VM>);
impl<VM: VMBinding> LinearScanObjectSize for DefaultObjectSize<VM> {
    #[inline(always)]
    fn size(object: ObjectReference) -> usize {
        VM::VMObjectModel::get_current_size(object)
    }
}

/// Region represents a memory region with a properly aligned address as its start and a fixed size for the region.
/// Region provides a set of utility methods, along with a RegionIterator that linearly scans at the step of a region.
pub trait Region: Copy + PartialEq + PartialOrd {
    const LOG_BYTES: usize;
    const BYTES: usize = 1 << Self::LOG_BYTES;

    /// Create a region from an address that is aligned to the region boundary. The method should panic if the address
    /// is not properly aligned to the region. For performance, this method should always be inlined.
    fn from_aligned_address(address: Address) -> Self;
    /// Return the start address of the region. For performance, this method should always be inlined.
    fn start(&self) -> Address;

    /// Create a region from an arbitrary address.
    #[inline(always)]
    fn from_unaligned_address(address: Address) -> Self {
        Self::from_aligned_address(Self::align(address))
    }

    /// Align the address to the region.
    #[inline(always)]
    fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }
    /// Check if an address is aligned to the region.
    #[inline(always)]
    fn is_aligned(address: Address) -> bool {
        address.is_aligned_to(Self::BYTES)
    }

    /// Return the end address of the region. Note that the end address is not in the region.
    #[inline(always)]
    fn end(&self) -> Address {
        self.start() + Self::BYTES
    }
    /// Return the next region after this one.
    #[inline(always)]
    fn next(&self) -> Self {
        self.next_nth(1)
    }
    /// Return the next nth region after this one.
    #[inline(always)]
    fn next_nth(&self, n: usize) -> Self {
        debug_assert!(self.start().as_usize() < usize::MAX - (n << Self::LOG_BYTES));
        Self::from_aligned_address(self.start() + (n << Self::LOG_BYTES))
    }
    /// Return the region that contains the object (by its cell address).
    #[inline(always)]
    fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self::from_unaligned_address(object.to_address::<VM>())
    }
}

pub struct RegionIterator<R: Region> {
    current: R,
    end: R,
}

impl<R: Region> RegionIterator<R> {
    pub fn new(start: R, end: R) -> Self {
        Self {
            current: start,
            end,
        }
    }
}

impl<R: Region> Iterator for RegionIterator<R> {
    type Item = R;

    fn next(&mut self) -> Option<R> {
        if self.current < self.end {
            let ret = self.current;
            self.current = self.current.next();
            Some(ret)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::constants::LOG_BYTES_IN_PAGE;

    const PAGE_SIZE: usize = 1 << LOG_BYTES_IN_PAGE;

    #[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
    struct Page(Address);

    impl Region for Page {
        const LOG_BYTES: usize = LOG_BYTES_IN_PAGE as usize;

        #[inline(always)]
        fn from_aligned_address(address: Address) -> Self {
            debug_assert!(address.is_aligned_to(Self::BYTES));
            Self(address)
        }

        #[inline(always)]
        fn start(&self) -> Address {
            self.0
        }
    }

    #[test]
    fn test_region_methods() {
        let addr4k = unsafe { Address::from_usize(PAGE_SIZE) };
        let addr4k1 = unsafe { Address::from_usize(PAGE_SIZE + 1) };

        // align
        debug_assert_eq!(Page::align(addr4k), addr4k);
        debug_assert_eq!(Page::align(addr4k1), addr4k);
        debug_assert!(Page::is_aligned(addr4k));
        debug_assert!(!Page::is_aligned(addr4k1));

        let page = Page::from_aligned_address(addr4k);
        // start/end
        debug_assert_eq!(page.start(), addr4k);
        debug_assert_eq!(page.end(), addr4k + PAGE_SIZE);
        // next
        debug_assert_eq!(page.next().start(), addr4k + PAGE_SIZE);
        debug_assert_eq!(page.next_nth(1).start(), addr4k + PAGE_SIZE);
        debug_assert_eq!(page.next_nth(2).start(), addr4k + 2 * PAGE_SIZE);
    }

    #[test]
    fn test_region_iterator_normal() {
        let addr4k = unsafe { Address::from_usize(PAGE_SIZE) };
        let page = Page::from_aligned_address(addr4k);
        let end_page = page.next_nth(5);

        let mut results = vec![];
        let iter = RegionIterator::new(page, end_page);
        for p in iter {
            results.push(p);
        }
        debug_assert_eq!(
            results,
            vec![
                page,
                page.next_nth(1),
                page.next_nth(2),
                page.next_nth(3),
                page.next_nth(4)
            ]
        );
    }

    #[test]
    fn test_region_iterator_same_start_end() {
        let addr4k = unsafe { Address::from_usize(PAGE_SIZE) };
        let page = Page::from_aligned_address(addr4k);

        let mut results = vec![];
        let iter = RegionIterator::new(page, page);
        for p in iter {
            results.push(p);
        }
        debug_assert_eq!(results, vec![]);
    }

    #[test]
    fn test_region_iterator_smaller_end() {
        let addr4k = unsafe { Address::from_usize(PAGE_SIZE) };
        let page = Page::from_aligned_address(addr4k);
        let end = Page::from_aligned_address(Address::ZERO);

        let mut results = vec![];
        let iter = RegionIterator::new(page, end);
        for p in iter {
            results.push(p);
        }
        debug_assert_eq!(results, vec![]);
    }
}
