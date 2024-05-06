use crate::util::Address;
use core::sync::atomic::*;
use num_traits::{FromPrimitive, ToPrimitive};
use num_traits::{Unsigned, WrappingAdd, WrappingSub, Zero};

/// Describes bits and log2 bits for the numbers.
/// If num_traits has this, we do not need our own implementation: <https://github.com/rust-num/num-traits/issues/247>
pub trait Bits {
    /// The size of this atomic type in bits.
    const BITS: u32;
    /// The size (in log2) of this atomic type in bits.
    const LOG2: u32;
}
macro_rules! impl_bits_trait {
    ($t: ty) => {
        impl Bits for $t {
            const BITS: u32 = <$t>::BITS;
            const LOG2: u32 = Self::BITS.trailing_zeros();
        }
    };
}
impl_bits_trait!(u8);
impl_bits_trait!(u16);
impl_bits_trait!(u32);
impl_bits_trait!(u64);
impl_bits_trait!(usize);

/// Describes bitwise operations.
/// If num_traits has this, we do not need our own implementation: <https://github.com/rust-num/num-traits/issues/232>
pub trait BitwiseOps {
    /// Perform bitwise and for two values.
    fn bitand(self, other: Self) -> Self;
    /// Perform bitwise or for two values.
    fn bitor(self, other: Self) -> Self;
    /// Perform bitwise xor for two values.
    fn bitxor(self, other: Self) -> Self;
    /// Perform bitwise invert (not) for the value.
    fn inv(self) -> Self;
}
macro_rules! impl_bitwise_ops_trait {
    ($t: ty) => {
        impl BitwiseOps for $t {
            fn bitand(self, other: Self) -> Self {
                self & other
            }
            fn bitor(self, other: Self) -> Self {
                self | other
            }
            fn bitxor(self, other: Self) -> Self {
                self ^ other
            }
            fn inv(self) -> Self {
                !self
            }
        }
    };
}
impl_bitwise_ops_trait!(u8);
impl_bitwise_ops_trait!(u16);
impl_bitwise_ops_trait!(u32);
impl_bitwise_ops_trait!(u64);
impl_bitwise_ops_trait!(usize);

/// The number type for accessing metadata.
/// It requires a few traits from num-traits and a few traits we defined above.
/// The methods in this trait are mostly about atomically accessing such types.
pub trait MetadataValue:
    Unsigned
    + Zero
    + WrappingAdd
    + WrappingSub
    + Bits
    + BitwiseOps
    + ToPrimitive
    + Copy
    + FromPrimitive
    + std::fmt::Display
    + std::fmt::Debug
{
    /// Non atomic load
    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    /// The caller also needs to be aware that the method is not thread safe, as it is a non-atomic operation.
    unsafe fn load(addr: Address) -> Self;

    /// Atomic load
    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn load_atomic(addr: Address, order: Ordering) -> Self;

    /// Non atomic store
    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    /// The caller also needs to be aware that the method is not thread safe, as it is a non-atomic operation.
    unsafe fn store(addr: Address, value: Self);

    /// Atomic store
    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    /// The caller also needs to be aware that the method is not thread safe, as it is a non-atomic operation.
    unsafe fn store_atomic(addr: Address, value: Self, order: Ordering);

    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn compare_exchange(
        addr: Address,
        current: Self,
        new: Self,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Self, Self>;

    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn fetch_add(addr: Address, value: Self, order: Ordering) -> Self;

    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn fetch_sub(addr: Address, value: Self, order: Ordering) -> Self;

    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn fetch_and(addr: Address, value: Self, order: Ordering) -> Self;

    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn fetch_or(addr: Address, value: Self, order: Ordering) -> Self;

    /// # Safety
    /// The caller needs to guarantee that the address is valid, and can be used as a pointer to the type.
    unsafe fn fetch_update<F>(
        addr: Address,
        set_order: Ordering,
        fetch_order: Ordering,
        f: F,
    ) -> Result<Self, Self>
    where
        F: FnMut(Self) -> Option<Self>;
}
macro_rules! impl_metadata_value_trait {
    ($non_atomic: ty, $atomic: ty) => {
        impl MetadataValue for $non_atomic {
            unsafe fn load(addr: Address) -> Self {
                addr.load::<$non_atomic>()
            }

            unsafe fn load_atomic(addr: Address, order: Ordering) -> Self {
                addr.as_ref::<$atomic>().load(order)
            }

            unsafe fn store(addr: Address, value: Self) {
                addr.store::<$non_atomic>(value)
            }

            unsafe fn store_atomic(addr: Address, value: Self, order: Ordering) {
                addr.as_ref::<$atomic>().store(value, order)
            }

            unsafe fn compare_exchange(
                addr: Address,
                current: Self,
                new: Self,
                success: Ordering,
                failure: Ordering,
            ) -> Result<Self, Self> {
                addr.as_ref::<$atomic>()
                    .compare_exchange(current, new, success, failure)
            }

            unsafe fn fetch_add(addr: Address, value: Self, order: Ordering) -> Self {
                addr.as_ref::<$atomic>().fetch_add(value, order)
            }

            unsafe fn fetch_sub(addr: Address, value: Self, order: Ordering) -> Self {
                addr.as_ref::<$atomic>().fetch_sub(value, order)
            }

            unsafe fn fetch_and(addr: Address, value: Self, order: Ordering) -> Self {
                addr.as_ref::<$atomic>().fetch_and(value, order)
            }

            unsafe fn fetch_or(addr: Address, value: Self, order: Ordering) -> Self {
                addr.as_ref::<$atomic>().fetch_or(value, order)
            }

            unsafe fn fetch_update<F>(
                addr: Address,
                set_order: Ordering,
                fetch_order: Ordering,
                f: F,
            ) -> Result<Self, Self>
            where
                F: FnMut(Self) -> Option<Self>,
            {
                addr.as_ref::<$atomic>()
                    .fetch_update(set_order, fetch_order, f)
            }
        }
    };
}
impl_metadata_value_trait!(u8, AtomicU8);
impl_metadata_value_trait!(u16, AtomicU16);
impl_metadata_value_trait!(u32, AtomicU32);
impl_metadata_value_trait!(u64, AtomicU64);
impl_metadata_value_trait!(usize, AtomicUsize);
