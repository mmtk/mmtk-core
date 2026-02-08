use atomic_traits::Atomic;
use bytemuck::NoUninit;

use std::fmt;
use std::mem;
use std::num::NonZeroUsize;
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
    pub const MAX: Self = Address(usize::MAX);

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

    /// Apply an signed offset to the address.
    pub const fn offset(self, offset: isize) -> Address {
        Address(self.0.wrapping_add_signed(offset))
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

    /// Returns an iterator which steps from this address to below the
    /// `end` address, in steps of `step` bytes.
    pub fn iter_to(&self, end: Address, step: usize) -> AddressIterator {
        AddressIterator {
            start: *self,
            end,
            step,
        }
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

/// Iterate addresses from a start address to below an end address,
/// with a given step size.
pub struct AddressIterator {
    start: Address,
    end: Address,
    step: usize,
}

impl Iterator for AddressIterator {
    type Item = Address;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            None
        } else {
            let current = self.start;
            self.start += self.step;
            Some(current)
        }
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

/// `ObjectReference` represents address for an object. Compared with `Address`, operations allowed
/// on `ObjectReference` are very limited. No address arithmetics are allowed for `ObjectReference`.
/// The idea is from the paper [Demystifying Magic: High-level Low-level Programming (VEE09)][FBC09]
/// and [JikesRVM].
///
/// In MMTk, `ObjectReference` holds a non-zero address, i.e. its **raw address**.  It must satisfy
/// the following requirements.
///
/// -   It uniquely references an MMTk object.
/// -   The address must be within the address range of the object it refers to.
/// -   The address must be word-aligned.
/// -   It must be efficient to access object metadata from an `ObjectReference`.
///
/// Each `ObjectReference` uniquely identifies exactly one MMTk object.  There is no "null
/// reference" (see below for details).
///
/// Conversely, each object has a unique (raw) address used for `ObjectReference`.  That address is
/// nominated by the VM binding right after an object is allocated in the MMTk heap (i.e. the
/// argument of [`crate::memory_manager::post_alloc`]).  The same address is used by all
/// `ObjectReference` instances that refer to that object until the object is moved, at which time
/// the VM binding shall choose another address to use as the `ObjectReference` of the new copy (in
/// [`crate::vm::ObjectModel::copy`] or [`crate::vm::ObjectModel::get_reference_when_copied_to`])
/// until the object is moved again.
///
/// In addition to the raw address, there are also two addresses related to each object allocated in
/// MMTk heap, namely **starting address** and **header address**.  See the
/// [`crate::vm::ObjectModel`] trait for their precise definition.
///
/// The VM binding may, in theory, pick any aligned address within the object, and it doesn't have
/// to be the starting address.  However, during tracing, MMTk will need to access object metadata
/// from a `ObjectReference`.  Particularly, it needs to identify reference fields, and query
/// information about the object, such as object size.  Such information is usually accessed from
/// object headers.  The choice of `ObjectReference` must make such accesses efficient.
///
/// Because the raw address is within the object, MMTk will also use the raw address to identify the
/// space or region (chunk, block, line, etc.) that contains the object, and to access side metadata
/// and the SFTMap.  If a VM binding needs to access side metadata directly (particularly, setting
/// the "valid-object (VO) bit" in allocation fast paths), it shall use the raw address to compute
/// the byte and bit address of the metadata bits.
///
/// # Notes
///
/// ## About VMs own concepts of "object references"
///
/// A runtime may define its own concept of "object references" differently from MMTk's
/// `ObjectReference` type.  It may define its object reference as
///
/// -   the starting address of an object,
/// -   an address inside an object,
/// -   an address at a certain offset outside an object,
/// -   a handle that points to an indirection table entry where a pointer to the object is held, or
/// -   anything else that refers to an object.
///
/// Regardless, when passing an `ObjectReference` value to MMTk through the API, MMTk expectes its
/// value to satisfy MMTk's definition.  This means MMTk's `ObjectReference` may not be the value
/// held in an object field.  Some VM bindings may need to do conversions when passing object
/// references to MMTk.  For example, adding an offset to the VM-level object reference so that the
/// resulting address is within the object.  When using handles, the VM binding may use the *pointer
/// stored in the entry* of the indirection table instead of the *pointer to the entry* itself as
/// MMTk-level `ObjectReference`.
///
/// ## About null references
///
/// An [`ObjectReference`] always refers to an object.  Some VMs have special values (such as `null`
/// in Java) that do not refer to any object.  Those values cannot be represented by
/// `ObjectReference`.  When scanning roots and object fields, the VM binding should ignore slots
/// that do not hold a reference to an object.  Specifically, [`crate::vm::slot::Slot::load`]
/// returns `Option<ObjectReference>`.  It can return `None` so that MMTk skips that slot.
///
/// `Option<ObjectReference>` should be used for the cases where a non-null object reference may or
/// may not exist,  That includes several API functions, including [`crate::vm::slot::Slot::load`].
/// [`ObjectReference`] is backed by `NonZeroUsize` which cannot be zero, and it has the
/// `#[repr(transparent)]` attribute. Thanks to [null pointer optimization (NPO)][NPO],
/// `Option<ObjectReference>` has the same size as `NonZeroUsize` and `usize`.
///
/// For the convenience of passing `Option<ObjectReference>` to and from native (C/C++) programs,
/// mmtk-core provides [`crate::util::api_util::NullableObjectReference`].
///
/// ## About the `VMSpace`
///
/// The `VMSpace` is managed by the VM binding.  The VM binding declare ranges of memory as part of
/// the `VMSpace`, but MMTk never allocates into it.  The VM binding allocates objects into the
/// `VMSpace` (usually by mapping boot-images), and refers to objects in the `VMSpace` using
/// `ObjectReference`s whose raw addresses point inside those objects (and must be word-aligned,
/// too).  MMTk will access metadata using methods of [`ObjectModel`] like other objects.  MMTk also
/// has side metadata available for objects in the `VMSpace`.
///
/// ## About `ObjectReference` pointing outside MMTk spaces
///
/// If a VM binding implements [`crate::vm::ActivePlan::vm_trace_object`], `ObjectReference` is
/// allowed to point to locations outside any MMTk spaces.  When tracing objects, such
/// `ObjectReference` values will be processed by `ActivePlan::vm_trace_object` so that the VM
/// binding can trace its own allocated objects during GC.  However, **this is an experimental
/// feature**, and may not interact well with other parts of MMTk.  Notably, MMTk will not allocate
/// side metadata for such `ObjectReference`, and attempts to access side metadata with a non-MMTk
/// `ObjectReference` will result in crash. Use with caution.
///
/// [FBC09]: https://dl.acm.org/doi/10.1145/1508293.1508305
/// [JikesRVM]: https://www.jikesrvm.org/
/// [`ObjectModel`]: crate::vm::ObjectModel
/// [NPO]: https://doc.rust-lang.org/std/option/index.html#representation
#[repr(transparent)]
#[derive(Copy, Clone, Eq, Hash, PartialOrd, Ord, PartialEq, NoUninit)]
pub struct ObjectReference(NonZeroUsize);

impl ObjectReference {
    /// The required minimal alignment for object reference. If the object reference's raw address is not aligned to this value,
    /// you will see an assertion failure in the debug build when constructing an object reference instance.
    pub const ALIGNMENT: usize = crate::util::constants::BYTES_IN_ADDRESS;

    /// Cast the object reference to its raw address.
    pub fn to_raw_address(self) -> Address {
        Address(self.0.get())
    }

    /// Cast a raw address to an object reference.
    ///
    /// If `addr` is 0, the result is `None`.
    pub fn from_raw_address(addr: Address) -> Option<ObjectReference> {
        debug_assert!(
            addr.is_aligned_to(Self::ALIGNMENT),
            "ObjectReference is required to be word aligned.  addr: {addr}"
        );
        NonZeroUsize::new(addr.0).map(ObjectReference)
    }

    /// Like `from_raw_address`, but assume `addr` is not zero.  This can be used to elide a check
    /// against zero for performance-critical code.
    ///
    /// # Safety
    ///
    /// This method assumes `addr` is not zero.  It should only be used in cases where we know at
    /// compile time that the input cannot be zero.  For example, if we compute the address by
    /// adding a positive offset to a non-zero address, we know the result must not be zero.
    pub unsafe fn from_raw_address_unchecked(addr: Address) -> ObjectReference {
        debug_assert!(!addr.is_zero());
        debug_assert!(
            addr.is_aligned_to(Self::ALIGNMENT),
            "ObjectReference is required to be word aligned.  addr: {addr}"
        );
        ObjectReference(NonZeroUsize::new_unchecked(addr.0))
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
        debug_assert!(!VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS || object_start == self.to_raw_address(), "The binding claims unified object reference address, but for object reference {}, ref_to_object_start() returns {}", self, object_start);
        debug_assert!(
            self.to_raw_address()
                >= object_start + VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND,
            "The invariant `object_ref >= object_start + OBJECT_REF_OFFSET_LOWER_BOUND` is violated. \
            object_ref: {}, object_start: {}, OBJECT_REF_OFFSET_LOWER_BOUND: {}",
            self.to_raw_address(),
            object_start,
            VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND,
        );
        object_start
    }

    /// Is the object reachable, determined by the policy?
    ///
    /// # Scope
    ///
    /// This method is primarily used during weak reference processing.  It can check if an object
    /// (particularly finalizable objects and objects pointed by weak references) has been reached
    /// by following strong references or weak references of higher strength.
    ///
    /// This method can also be used during tracing for debug purposes.
    ///
    /// When called at other times, particularly during mutator time, the behavior is specific to
    /// the implementation of the plan and policy due to their strategies of metadata clean-up.  If
    /// the VM needs to know if any given reference is still valid, it should instead use the valid
    /// object bit (VO-bit) metadata which is enabled by the Cargo feature "vo_bit".
    ///
    /// # Return value
    ///
    /// It returns `true` if one of the following is true:
    ///
    /// 1.  The object has been traced (i.e. reached) since tracing started.
    /// 2.  The policy conservatively considers the object reachable even though it has not been
    ///     traced.
    ///     -   Particularly, if the plan is generational, this method will return `true` if the
    ///         object is mature during nursery GC.
    ///
    /// Due to the conservativeness, if this method returns `true`, it does not necessarily mean the
    /// object must be reachable from roots.  In generational GC, mature objects can be unreachable
    /// from roots while the GC chooses not to reclaim their memory during nursery GC. Conversely,
    /// all young objects reachable from the remembered set are retained even though some mature
    /// objects in the remembered set can be unreachable in the first place.  (This is known as
    /// *nepotism* in GC literature.)
    ///
    /// Note: Objects in ImmortalSpace may have `is_live = true` but are actually unreachable.
    pub fn is_reachable(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(self.to_raw_address()) }.is_reachable(self)
    }

    /// Is the object live, determined by the policy?
    pub fn is_live(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(self.to_raw_address()) }.is_live(self)
    }

    /// Can the object be moved?
    pub fn is_movable(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(self.to_raw_address()) }.is_movable()
    }

    /// Get forwarding pointer if the object is forwarded.
    pub fn get_forwarded_object(self) -> Option<Self> {
        unsafe { SFT_MAP.get_unchecked(self.to_raw_address()) }.get_forwarded_object(self)
    }

    /// Is the object in any MMTk spaces?
    pub fn is_in_any_space(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(self.to_raw_address()) }.is_in_space(self)
    }

    /// Is the object sane?
    #[cfg(feature = "sanity")]
    pub fn is_sane(self) -> bool {
        unsafe { SFT_MAP.get_unchecked(self.to_raw_address()) }.is_sane()
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
