use atomic_traits::Atomic;
use bytemuck::NoUninit;

use std::fmt;
use std::mem;
use std::ops::*;
use std::sync::atomic::Ordering;

use crate::mmtk::{MMAPPER, SFT_MAP};

/// size in bytes
pub type ByteSize = usize;
/// offset in byte
pub type ByteOffset = isize;

/// Address represents an arbitrary address. This is designed to represent
/// address and do address arithmetic mostly in a safe way, and to allow
/// mark some operations as unsafe. This type needs to be zero overhead
/// (memory wise and time wise). The idea is from the paper
/// High-level Low-level Programming (VEE09) and JikesRVM.
#[repr(transparent)]
#[derive(Copy, Clone, Eq, Hash, PartialOrd, Ord, PartialEq, NoUninit)]
pub struct Address(usize);

/// Address + ByteSize (positive)
impl Add<ByteSize> for Address {
    type Output = Address;
    fn add(self, offset: ByteSize) -> Address {
        Address(self.0 + offset)
    }
}

/// Address += ByteSize (positive)
impl AddAssign<ByteSize> for Address {
    fn add_assign(&mut self, offset: ByteSize) {
        self.0 += offset;
    }
}

/// Address + ByteOffset (positive or negative)
impl Add<ByteOffset> for Address {
    type Output = Address;
    fn add(self, offset: ByteOffset) -> Address {
        Address((self.0 as isize + offset) as usize)
    }
}

/// Address += ByteOffset (positive or negative)
impl AddAssign<ByteOffset> for Address {
    fn add_assign(&mut self, offset: ByteOffset) {
        self.0 = (self.0 as isize + offset) as usize
    }
}

/// Address - ByteSize (positive)
impl Sub<ByteSize> for Address {
    type Output = Address;
    fn sub(self, offset: ByteSize) -> Address {
        Address(self.0 - offset)
    }
}

/// Address -= ByteSize (positive)
impl SubAssign<ByteSize> for Address {
    fn sub_assign(&mut self, offset: ByteSize) {
        self.0 -= offset;
    }
}

/// Address - Address (the first address must be higher)
impl Sub<Address> for Address {
    type Output = ByteSize;
    fn sub(self, other: Address) -> ByteSize {
        debug_assert!(
            self.0 >= other.0,
            "for (addr_a - addr_b), a({}) needs to be larger than b({})",
            self,
            other
        );
        self.0 - other.0
    }
}

/// Address & mask
impl BitAnd<usize> for Address {
    type Output = usize;
    fn bitand(self, other: usize) -> usize {
        self.0 & other
    }
}
// Be careful about the return type here. Address & u8 = u8
// This is different from Address | u8 = usize
impl BitAnd<u8> for Address {
    type Output = u8;
    fn bitand(self, other: u8) -> u8 {
        (self.0 as u8) & other
    }
}

/// Address | mask
impl BitOr<usize> for Address {
    type Output = usize;
    fn bitor(self, other: usize) -> usize {
        self.0 | other
    }
}
// Be careful about the return type here. Address | u8 = size
// This is different from Address & u8 = u8
impl BitOr<u8> for Address {
    type Output = usize;
    fn bitor(self, other: u8) -> usize {
        self.0 | (other as usize)
    }
}

/// Address >> shift (get an index)
impl Shr<usize> for Address {
    type Output = usize;
    fn shr(self, shift: usize) -> usize {
        self.0 >> shift
    }
}

/// Address << shift (get an index)
impl Shl<usize> for Address {
    type Output = usize;
    fn shl(self, shift: usize) -> usize {
        self.0 << shift
    }
}

impl Address {
    /// The lowest possible address.
    pub const ZERO: Self = Address(0);
    /// The highest possible address.
    pub const MAX: Self = Address(usize::max_value());

    /// creates Address from a pointer
    pub fn from_ptr<T>(ptr: *const T) -> Address {
        Address(ptr as usize)
    }

    /// creates Address from a Rust reference
    pub fn from_ref<T>(r: &T) -> Address {
        Address(r as *const T as usize)
    }

    /// creates Address from a mutable pointer
    pub fn from_mut_ptr<T>(ptr: *mut T) -> Address {
        Address(ptr as usize)
    }

    /// creates a null Address (0)
    /// # Safety
    /// It is unsafe and the user needs to be aware that they are creating an invalid address.
    /// The zero address should only be used as unininitialized or sentinel values in performance critical code (where you dont want to use `Option<Address>`).
    pub const unsafe fn zero() -> Address {
        Address(0)
    }

    /// creates an Address of (usize::MAX)
    /// # Safety
    /// It is unsafe and the user needs to be aware that they are creating an invalid address.
    /// The max address should only be used as unininitialized or sentinel values in performance critical code (where you dont want to use `Option<Address>`).
    pub unsafe fn max() -> Address {
        use std::usize;
        Address(usize::MAX)
    }

    /// creates an arbitrary Address
    /// # Safety
    /// It is unsafe and the user needs to be aware that they may create an invalid address.
    /// This creates arbitrary addresses which may not be valid. This should only be used for hard-coded addresses. Any other uses of this function could be
    /// replaced with more proper alternatives.
    pub const unsafe fn from_usize(raw: usize) -> Address {
        Address(raw)
    }

    /// shifts the address by N T-typed objects (returns addr + N * size_of(T))
    pub fn shift<T>(self, offset: isize) -> Self {
        self + mem::size_of::<T>() as isize * offset
    }

    // These const functions are duplicated with the operator traits. But we need them,
    // as we need them to declare constants.

    /// Get the number of bytes between two addresses. The current address needs to be higher than the other address.
    pub const fn get_extent(self, other: Address) -> ByteSize {
        self.0 - other.0
    }

    /// Get the offset from `other` to `self`. The result is negative is `self` is lower than `other`.
    pub const fn get_offset(self, other: Address) -> ByteOffset {
        self.0 as isize - other.0 as isize
    }

    // We implemented the Add trait but we still keep this add function.
    // The add() function is const fn, and we can use it to declare Address constants.
    // The Add trait function cannot be const.
    #[allow(clippy::should_implement_trait)]
    /// Add an offset to the address.
    pub const fn add(self, size: usize) -> Address {
        Address(self.0 + size)
    }

    // We implemented the Sub trait but we still keep this sub function.
    // The sub() function is const fn, and we can use it to declare Address constants.
    // The Sub trait function cannot be const.
    #[allow(clippy::should_implement_trait)]
    /// Subtract an offset from the address.
    pub const fn sub(self, size: usize) -> Address {
        Address(self.0 - size)
    }

    /// Bitwise 'and' with a mask.
    pub const fn and(self, mask: usize) -> usize {
        self.0 & mask
    }

    /// Perform a saturating subtract on the Address
    pub const fn saturating_sub(self, size: usize) -> Address {
        Address(self.0.saturating_sub(size))
    }

    /// loads a value of type T from the address
    /// # Safety
    /// This could throw a segment fault if the address is invalid
    pub unsafe fn load<T: Copy>(self) -> T {
        *(self.0 as *mut T)
    }

    /// stores a value of type T to the address
    /// # Safety
    /// This could throw a segment fault if the address is invalid
    pub unsafe fn store<T>(self, value: T) {
        // We use a ptr.write() operation as directly setting the pointer would drop the old value
        // which may result in unexpected behaviour
        (self.0 as *mut T).write(value);
    }

    /// atomic operation: load
    /// # Safety
    /// This could throw a segment fault if the address is invalid
    pub unsafe fn atomic_load<T: Atomic>(self, order: Ordering) -> T::Type {
        let loc = &*(self.0 as *const T);
        loc.load(order)
    }

    /// atomic operation: store
    /// # Safety
    /// This could throw a segment fault if the address is invalid
    pub unsafe fn atomic_store<T: Atomic>(self, val: T::Type, order: Ordering) {
        let loc = &*(self.0 as *const T);
        loc.store(val, order)
    }

    /// atomic operation: compare and exchange usize
    /// # Safety
    /// This could throw a segment fault if the address is invalid
    pub unsafe fn compare_exchange<T: Atomic>(
        self,
        old: T::Type,
        new: T::Type,
        success: Ordering,
        failure: Ordering,
    ) -> Result<T::Type, T::Type> {
        let loc = &*(self.0 as *const T);
        loc.compare_exchange(old, new, success, failure)
    }

    /// is this address zero?
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// aligns up the address to the given alignment
    pub const fn align_up(self, align: ByteSize) -> Address {
        use crate::util::conversions;
        Address(conversions::raw_align_up(self.0, align))
    }

    /// aligns down the address to the given alignment
    pub const fn align_down(self, align: ByteSize) -> Address {
        use crate::util::conversions;
        Address(conversions::raw_align_down(self.0, align))
    }

    /// is this address aligned to the given alignment
    pub const fn is_aligned_to(self, align: usize) -> bool {
        use crate::util::conversions;
        conversions::raw_is_aligned(self.0, align)
    }

    /// converts the Address to a pointer
    pub fn to_ptr<T>(self) -> *const T {
        self.0 as *const T
    }

    /// converts the Address to a mutable pointer
    pub fn to_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    /// converts the Address to a Rust reference
    ///
    /// # Safety
    /// The caller must guarantee the address actually points to a Rust object.
    pub unsafe fn as_ref<'a, T>(self) -> &'a T {
        &*self.to_mut_ptr()
    }

    /// converts the Address to a mutable Rust reference
    ///
    /// # Safety
    /// The caller must guarantee the address actually points to a Rust object.
    pub unsafe fn as_mut_ref<'a, T>(self) -> &'a mut T {
        &mut *self.to_mut_ptr()
    }

    /// converts the Address to a pointer-sized integer
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// returns the chunk index for this address
    pub fn chunk_index(self) -> usize {
        use crate::util::conversions;
        conversions::address_to_chunk_index(self)
    }

    /// return true if the referenced memory is mapped
    pub fn is_mapped(self) -> bool {
        if self.0 == 0 {
            false
        } else {
            MMAPPER.is_mapped_address(self)
        }
    }

    /// Returns the intersection of the two address ranges. The returned range could
    /// be empty if there is no intersection between the ranges.
    pub fn range_intersection(r1: &Range<Address>, r2: &Range<Address>) -> Range<Address> {
        r1.start.max(r2.start)..r1.end.min(r2.end)
    }
}

/// allows print Address as upper-case hex value
impl fmt::UpperHex for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:X}", self.0)
    }
}

/// allows print Address as lower-case hex value
impl fmt::LowerHex for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

/// allows Display format the Address (as upper-case hex value with 0x prefix)
impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// allows Debug format the Address (as upper-case hex value with 0x prefix)
impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl std::str::FromStr for Address {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw: usize = s.parse()?;
        Ok(Address(raw))
    }
}

#[cfg(test)]
mod tests {
    use crate::util::Address;

    #[test]
    fn align_up() {
        unsafe {
            assert_eq!(
                Address::from_usize(0x10).align_up(0x10),
                Address::from_usize(0x10)
            );
            assert_eq!(
                Address::from_usize(0x11).align_up(0x10),
                Address::from_usize(0x20)
            );
            assert_eq!(
                Address::from_usize(0x20).align_up(0x10),
                Address::from_usize(0x20)
            );
        }
    }

    #[test]
    fn align_down() {
        unsafe {
            assert_eq!(
                Address::from_usize(0x10).align_down(0x10),
                Address::from_usize(0x10)
            );
            assert_eq!(
                Address::from_usize(0x11).align_down(0x10),
                Address::from_usize(0x10)
            );
            assert_eq!(
                Address::from_usize(0x20).align_down(0x10),
                Address::from_usize(0x20)
            );
        }
    }

    #[test]
    fn is_aligned_to() {
        unsafe {
            assert!(Address::from_usize(0x10).is_aligned_to(0x10));
            assert!(!Address::from_usize(0x11).is_aligned_to(0x10));
            assert!(Address::from_usize(0x10).is_aligned_to(0x8));
            assert!(!Address::from_usize(0x10).is_aligned_to(0x20));
        }
    }

    #[test]
    fn bit_and() {
        unsafe {
            assert_eq!(
                Address::from_usize(0b1111_1111_1100usize) & 0b1010u8,
                0b1000u8
            );
            assert_eq!(
                Address::from_usize(0b1111_1111_1100usize) & 0b1000_0000_1010usize,
                0b1000_0000_1000usize
            );
        }
    }

    #[test]
    fn bit_or() {
        unsafe {
            assert_eq!(
                Address::from_usize(0b1111_1111_1100usize) | 0b1010u8,
                0b1111_1111_1110usize
            );
            assert_eq!(
                Address::from_usize(0b1111_1111_1100usize) | 0b1000_0000_1010usize,
                0b1111_1111_1110usize
            );
        }
    }
}

use crate::vm::VMBinding;

/// ObjectReference represents address for an object. Compared with Address,
/// operations allowed on ObjectReference are very limited. No address arithmetics
/// are allowed for ObjectReference. The idea is from the paper
/// High-level Low-level Programming (VEE09) and JikesRVM.
///
/// A runtime may define its "object references" differently. It may define an object reference as
/// the address of an object, a handle that points to an indirection table entry where a pointer to
/// the object is held, or anything else. Regardless, MMTk expects each object reference to have a
/// pointer to the object (an address) in each object reference, and that address should be used
/// for this `ObjectReference` type.
///
/// We currently do not allow an opaque `ObjectReference` type for which a binding can define
/// their layout. We now only allow a binding to define their semantics through a set of
/// methods in [`crate::vm::ObjectModel`]. Major refactoring is needed in MMTk to allow
/// the opaque `ObjectReference` type, and we haven't seen a use case for now.
#[repr(transparent)]
#[derive(Copy, Clone, Eq, Hash, PartialOrd, Ord, PartialEq, NoUninit)]
pub struct ObjectReference(usize);

impl ObjectReference {
    /// The null object reference, represented as zero.
    pub const NULL: ObjectReference = ObjectReference(0);

    /// Cast the object reference to its raw address. This method is mostly for the convinience of a binding.
    ///
    /// MMTk should not make any assumption on the actual location of the address with the object reference.
    /// MMTk should not assume the address returned by this method is in our allocation. For the purposes of
    /// setting object metadata, MMTk should use [`crate::vm::ObjectModel::ref_to_address()`] or [`crate::vm::ObjectModel::ref_to_header()`].
    pub fn to_raw_address(self) -> Address {
        Address(self.0)
    }

    /// Cast a raw address to an object reference. This method is mostly for the convinience of a binding.
    /// This is how a binding creates `ObjectReference` instances.
    ///
    /// MMTk should not assume an arbitrary address can be turned into an object reference. MMTk can use [`crate::vm::ObjectModel::address_to_ref()`]
    /// to turn addresses that are from [`crate::vm::ObjectModel::ref_to_address()`] back to object.
    pub fn from_raw_address(addr: Address) -> ObjectReference {
        ObjectReference(addr.0)
    }

    /// Get the in-heap address from an object reference. This method is used by MMTk to get an in-heap address
    /// for an object reference. This method is syntactic sugar for [`crate::vm::ObjectModel::ref_to_address`]. See the
    /// comments on [`crate::vm::ObjectModel::ref_to_address`].
    pub fn to_address<VM: VMBinding>(self) -> Address {
        use crate::vm::ObjectModel;
        let to_address = VM::VMObjectModel::ref_to_address(self);
        debug_assert!(!VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS || to_address == self.to_raw_address(), "The binding claims unified object reference address, but for object reference {}, ref_to_address() returns {}", self, to_address);
        to_address
    }

    /// Get the header base address from an object reference. This method is used by MMTk to get a base address for the
    /// object header, and access the object header. This method is syntactic sugar for [`crate::vm::ObjectModel::ref_to_header`].
    /// See the comments on [`crate::vm::ObjectModel::ref_to_header`].
    pub fn to_header<VM: VMBinding>(self) -> Address {
        use crate::vm::ObjectModel;
        VM::VMObjectModel::ref_to_header(self)
    }

    /// Get the start of the allocation address for the object. This method is used by MMTk to get the start of the allocation
    /// address originally returned from [`crate::memory_manager::alloc`] for the object.
    /// This method is syntactic sugar for [`crate::vm::ObjectModel::ref_to_object_start`]. See comments on [`crate::vm::ObjectModel::ref_to_object_start`].
    pub fn to_object_start<VM: VMBinding>(self) -> Address {
        use crate::vm::ObjectModel;
        let object_start = VM::VMObjectModel::ref_to_object_start(self);
        debug_assert!(!VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS || object_start == self.to_raw_address(), "The binding claims unified object reference address, but for object reference {}, ref_to_address() returns {}", self, object_start);
        object_start
    }

    /// Get the object reference from an address that is returned from [`crate::util::address::ObjectReference::to_address`]
    /// or [`crate::vm::ObjectModel::ref_to_address`]. This method is syntactic sugar for [`crate::vm::ObjectModel::address_to_ref`].
    /// See the comments on [`crate::vm::ObjectModel::address_to_ref`].
    pub fn from_address<VM: VMBinding>(addr: Address) -> ObjectReference {
        use crate::vm::ObjectModel;
        let obj = VM::VMObjectModel::address_to_ref(addr);
        debug_assert!(!VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS || addr == obj.to_raw_address(), "The binding claims unified object reference address, but for address {}, address_to_ref() returns {}", addr, obj);
        obj
    }

    /// is this object reference null reference?
    pub fn is_null(self) -> bool {
        self.0 == 0
    }

    /// returns the ObjectReference
    pub fn value(self) -> usize {
        self.0
    }

    /// Is the object reachable, determined by the policy?
    /// Note: Objects in ImmortalSpace may have `is_live = true` but are actually unreachable.
    pub fn is_reachable(self) -> bool {
        if self.is_null() {
            false
        } else {
            unsafe { SFT_MAP.get_unchecked(Address(self.0)) }.is_reachable(self)
        }
    }

    /// Is the object live, determined by the policy?
    pub fn is_live(self) -> bool {
        if self.0 == 0 {
            false
        } else {
            unsafe { SFT_MAP.get_unchecked(Address(self.0)) }.is_live(self)
        }
    }

    /// Can the object be moved?
    pub fn is_movable(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(Address(self.0)) }.is_movable()
    }

    /// Get forwarding pointer if the object is forwarded.
    pub fn get_forwarded_object(self) -> Option<Self> {
        unsafe { SFT_MAP.get_unchecked(Address(self.0)) }.get_forwarded_object(self)
    }

    /// Is the object in any MMTk spaces?
    pub fn is_in_any_space(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(Address(self.0)) }.is_in_space(self)
    }

    /// Is the object sane?
    #[cfg(feature = "sanity")]
    pub fn is_sane(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(Address(self.0)) }.is_sane()
    }
}

/// allows print Address as upper-case hex value
impl fmt::UpperHex for ObjectReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:X}", self.0)
    }
}

/// allows print Address as lower-case hex value
impl fmt::LowerHex for ObjectReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

/// allows Display format the Address (as upper-case hex value with 0x prefix)
impl fmt::Display for ObjectReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// allows Debug format the Address (as upper-case hex value with 0x prefix)
impl fmt::Debug for ObjectReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}
