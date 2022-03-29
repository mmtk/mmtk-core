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
                alloc_bit::is_alloced_object(self.cursor)
            } else {
                unsafe { alloc_bit::is_alloced_object_unsafe(self.cursor) }
            };

            if is_object {
                let object = unsafe { self.cursor.to_object_reference() };
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
pub trait Region: Copy + PartialEq + PartialOrd + From<Address> + Into<Address> {
    const LOG_BYTES: usize;
    const BYTES: usize = 1 << Self::LOG_BYTES;

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
    /// Return the start address of the region.
    #[inline(always)]
    fn start(&self) -> Address {
        (*self).into()
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
        Self::from(self.start() + (n << Self::LOG_BYTES))
    }
    /// Return the region that contains the object (by its cell address).
    #[inline(always)]
    fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self::from(VM::VMObjectModel::ref_to_address(object).align_down(Self::BYTES))
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
        let next = self.current.next();
        if next < self.end {
            self.current = next;
            Some(next)
        } else {
            None
        }
    }
}