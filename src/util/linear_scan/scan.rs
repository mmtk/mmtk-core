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
pub struct LinearScanIterator<
    VM: VMBinding,
    S: LinearScanObjectSize,
    const ATOMIC_LOAD_ALLOC_BIT: bool,
> {
    start: Address,
    end: Address,
    cursor: Address,
    _p: PhantomData<(VM, S)>,
}

impl<VM: VMBinding, S: LinearScanObjectSize, const ATOMIC_LOAD_ALLOC_BIT: bool>
    LinearScanIterator<VM, S, ATOMIC_LOAD_ALLOC_BIT>
{
    /// Create an iterator for the address range. The caller must ensure
    /// that the alloc bit metadata is mapped for the address range.
    pub fn new(start: Address, end: Address) -> Self {
        debug_assert!(start < end);
        LinearScanIterator {
            start,
            end,
            cursor: start,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, S: LinearScanObjectSize, const ATOMIC_LOAD_ALLOC_BIT: bool> std::iter::Iterator
    for LinearScanIterator<VM, S, ATOMIC_LOAD_ALLOC_BIT>
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
    fn size(object: ObjectReference) -> usize {
        VM::VMObjectModel::get_current_size(object)
    }
}
