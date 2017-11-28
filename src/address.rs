use std::cmp;
use std::fmt;
use std::mem;
use std::ops::*;

/// size in bytes
pub type ByteSize = usize;
/// offset in byte
pub type ByteOffset = isize;

/// Address represents an arbitrary address. This is designed to represent
/// address and do address arithmetic mostly in a safe way, and to allow
/// mark some operations as unsafe. This type needs to be zero overhead
/// (memory wise and time wise). The idea is from the paper
/// High-level Low-level Programming (VEE09) and JikesRVM.
#[repr(C)]
#[derive(Copy, Clone, Eq, Hash)]
pub struct Address(pub usize);

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
        debug_assert!(self.0 >= other.0, "for (addr_a - addr_b), a needs to be larger than b");
        self.0 - other.0
    }
}

impl Address {
    /// creates Address from a pointer
    #[inline(always)]
    pub fn from_ptr<T>(ptr: *const T) -> Address {
        unsafe { mem::transmute(ptr) }
    }

    #[inline(always)]
    pub fn from_ref<T>(r: &T) -> Address {
        unsafe { mem::transmute(r) }
    }

    /// creates Address from a mutable pointer
    #[inline(always)]
    pub fn from_mut_ptr<T>(ptr: *mut T) -> Address {
        unsafe { mem::transmute(ptr) }
    }

    /// creates a null Address (0)
    /// It is unsafe and the user needs to be aware that they are creating an invalid address.
    #[inline(always)]
    pub unsafe fn zero() -> Address {
        Address(0)
    }

    /// creates an Address of (usize::MAX)
    /// It is unsafe and the user needs to be aware that they are creating an invalid address.
    #[inline(always)]
    pub unsafe fn max() -> Address {
        use std::usize;
        Address(usize::MAX)
    }

    /// creates an arbitrary Address
    /// It is unsafe and the user needs to be aware that they may create an invalid address.
    #[inline(always)]
    pub unsafe fn from_usize(raw: usize) -> Address {
        Address(raw)
    }

    /// shifts the address by N T-typed objects (returns addr + N * size_of(T))
    #[inline(always)]
    pub fn shift<T>(self, offset: isize) -> Self {
        self + mem::size_of::<T>() as isize * offset
    }

    /// loads a value of type T from the address
    #[inline(always)]
    pub unsafe fn load<T: Copy>(&self) -> T {
        *(self.0 as *mut T)
    }

    /// stores a value of type T to the address
    #[inline(always)]
    pub unsafe fn store<T>(&self, value: T) {
        *(self.0 as *mut T) = value;
    }

    // commented out the function due to the fact that Rust does not have non-64bits atomic types
    // Issue #51

    //    /// loads a value of type T from the address with specified memory order
    //    #[inline(always)]
    //    pub unsafe fn load_order<T: Copy> (&self, order: Ordering) -> T {
    //        let atomic_ptr : AtomicPtr<T> = mem::transmute(self.0);
    //        *atomic_ptr.load(order)
    //    }

    //    /// stores a value of type T to the address with specified memory order
    //    #[inline(always)]
    //    pub unsafe fn store_order<T: Copy> (&self, mut value: T, order: Ordering) {
    //        let atomic_ptr : AtomicPtr<T> = mem::transmute(self.0);
    //        atomic_ptr.store(&mut value, order)
    //    }

    /// is this address zero?
    #[inline(always)]
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// aligns up the address to the given alignment
    #[inline(always)]
    pub fn align_up(&self, align: ByteSize) -> Address {
        Address((self.0 + align - 1) & !(align - 1))
    }

    /// is this address aligned to the given alignment
    pub fn is_aligned_to(&self, align: usize) -> bool {
        self.0 % align == 0
    }

    /// converts the Address into an ObjectReference
    /// Since we would expect ObjectReferences point to valid objects,
    /// but an arbitrary Address may reside an object, this conversion is unsafe,
    /// and it is the user's responsibility to ensure the safety.
    #[inline(always)]
    pub unsafe fn to_object_reference(&self) -> ObjectReference {
        mem::transmute(self.0)
    }

    /// converts the Address to a pointer
    #[inline(always)]
    pub fn to_ptr<T>(&self) -> *const T {
        unsafe { mem::transmute(self.0) }
    }

    /// converts the Address to a mutable pointer
    #[inline(always)]
    pub fn to_ptr_mut<T>(&self) -> *mut T {
        unsafe { mem::transmute(self.0) }
    }

    /// converts the Address to a pointer-sized integer
    #[inline(always)]
    pub fn as_usize(&self) -> usize {
        self.0
    }
}

/// allows comparison between Address
impl PartialOrd for Address {
    #[inline(always)]
    fn partial_cmp(&self, other: &Address) -> Option<cmp::Ordering> {
        Some(self.0.cmp(&other.0))
    }
}

/// allows equal test between Address
impl PartialEq for Address {
    #[inline(always)]
    fn eq(&self, other: &Address) -> bool {
        self.0 == other.0
    }
    #[inline(always)]
    fn ne(&self, other: &Address) -> bool {
        self.0 != other.0
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
        write!(f, "0x{:X}", self.0)
    }
}

/// allows Debug format the Address (as upper-case hex value with 0x prefix)
impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "0x{:X}", self.0)
    }
}

/// ObjectReference represents address for an object. Compared with Address,
/// operations allowed on ObjectReference are very limited. No address arithmetics
/// are allowed for ObjectReference. The idea is from the paper
/// High-level Low-level Programming (VEE09) and JikesRVM.
#[derive(Copy, Clone, Eq, Hash)]
pub struct ObjectReference(usize);

impl ObjectReference {
    /// converts the ObjectReference to an Address
    #[inline(always)]
    pub fn to_address(&self) -> Address {
        Address(self.0)
    }

    /// is this object reference null reference?
    #[inline(always)]
    pub fn is_null(&self) -> bool {
        self.0 != 0
    }

    /// returns the ObjectReference
    pub fn value(&self) -> usize {
        self.0
    }
}

/// allows equal test between Address
impl PartialEq for ObjectReference {
    #[inline(always)]
    fn eq(&self, other: &ObjectReference) -> bool {
        self.0 == other.0
    }
    #[inline(always)]
    fn ne(&self, other: &ObjectReference) -> bool {
        self.0 != other.0
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
        write!(f, "0x{:X}", self.0)
    }
}

/// allows Debug format the Address (as upper-case hex value with 0x prefix)
impl fmt::Debug for ObjectReference {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "0x{:X}", self.0)
    }
}
