use core::sync::atomic::*;
use num_traits::Unsigned;
use num_traits::{FromPrimitive, ToPrimitive};

/// Describes bits and log2 bits for the numbers.
/// If num_traits has this, we do not need our own implementation: https://github.com/rust-num/num-traits/issues/247
pub trait Bits {
    const BITS: u32 = 1 << Self::LOG2;
    const LOG2: u32;
}
macro_rules! impl_bits_trait {
    ($t: ty, $l: expr) => {
        impl Bits for $t {
            const LOG2: u32 = $l;
        }
    }
}
impl_bits_trait!(u8, 3);
impl_bits_trait!(u16, 4);
impl_bits_trait!(u32, 5);
impl_bits_trait!(u64, 6);

/// Atomic trait used for metadata.
/// Ideally we should use atomic_traits or atomic. However, for those traits,
/// their associate non-atomic type is a general type rather than a type of numbers.
pub trait MetadataAtomic: Sized {
    type NonAtomicType: MetadataValue;

    fn load(&self, order: Ordering) -> Self::NonAtomicType;
    fn store(&self, value: Self::NonAtomicType, order: Ordering);
    fn compare_exchange(
        &self,
        current: Self::NonAtomicType,
        new: Self::NonAtomicType,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Self::NonAtomicType, Self::NonAtomicType>;
    fn fetch_add(&self, value: Self::NonAtomicType, order: Ordering) -> Self::NonAtomicType;
    fn fetch_sub(&self, value: Self::NonAtomicType, order: Ordering) -> Self::NonAtomicType;
    fn fetch_update<F>(
        &self,
        set_order: Ordering,
        fetch_order: Ordering,
        f: F,
    ) -> Result<Self::NonAtomicType, Self::NonAtomicType>
    where
        F: FnMut(Self::NonAtomicType) -> Option<Self::NonAtomicType>;
}
macro_rules! impl_atomic_trait {
    ($atomic:ty, $non_atomic:ty) => {
        impl MetadataAtomic for $atomic {
            type NonAtomicType = $non_atomic;

            #[inline]
            fn load(&self, order: Ordering) -> Self::NonAtomicType {
                <$atomic>::load(self, order)
            }

            #[inline]
            fn store(&self, value: Self::NonAtomicType, order: Ordering) {
                <$atomic>::store(self, value, order)
            }

            #[inline]
            fn compare_exchange(
                &self,
                current: Self::NonAtomicType,
                new: Self::NonAtomicType,
                success: Ordering,
                failure: Ordering,
            ) -> Result<Self::NonAtomicType, Self::NonAtomicType> {
                <$atomic>::compare_exchange(
                    self,
                    current,
                    new,
                    success,
                    failure,
                )
            }

            #[inline]
            fn fetch_add(&self, value: Self::NonAtomicType, order: Ordering) -> Self::NonAtomicType{
                <$atomic>::fetch_add(self, value, order)
            }

            #[inline]
            fn fetch_sub(&self, value: Self::NonAtomicType, order: Ordering) -> Self::NonAtomicType{
                <$atomic>::fetch_sub(self, value, order)
            }

            #[inline]
            fn fetch_update<F>(
                &self,
                set_order: Ordering,
                fetch_order: Ordering,
                f: F,
            ) -> Result<Self::NonAtomicType, Self::NonAtomicType>
            where
                F: FnMut(Self::NonAtomicType) -> Option<Self::NonAtomicType> {
                <$atomic>::fetch_update(self, set_order, fetch_order, f)
            }
        }
    }
}
impl_atomic_trait!(AtomicU8, u8);
impl_atomic_trait!(AtomicU16, u16);
impl_atomic_trait!(AtomicU32, u32);
impl_atomic_trait!(AtomicU64, u64);

/// The number type for load/store metadata.
pub trait MetadataValue: Unsigned + Bits + ToPrimitive + Copy + FromPrimitive {
    type AtomicType: MetadataAtomic;
}
macro_rules! impl_metadata_value_trait {
    ($non_atomic: ty, $atomic: ty) => {
        impl MetadataValue for $non_atomic {
            type AtomicType = $atomic;
        }
    }
}
impl_metadata_value_trait!(u8, AtomicU8);
impl_metadata_value_trait!(u16, AtomicU16);
impl_metadata_value_trait!(u32, AtomicU32);
impl_metadata_value_trait!(u64, AtomicU64);
