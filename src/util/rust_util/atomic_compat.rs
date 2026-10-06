//! Compatibility shims for atomic operations whose preferred API is newer than our MSRV.

use std::sync::atomic::*;

/// Provides `fetch_update` for std atomic types without triggering the deprecation warning.
///
/// `fetch_update` on std atomic types is deprecated in favor of `try_update`, but `try_update` is
/// only stable since Rust 1.95. Once our MSRV reaches 1.95, remove this trait and replace
/// `fetch_update_compat` calls with `try_update`.
pub(crate) trait AtomicFetchUpdate {
    /// The underlying non-atomic value type.
    type Value;

    /// Same as `fetch_update` (or `try_update`) on std atomic types.
    fn fetch_update_compat<F>(
        &self,
        set_order: Ordering,
        fetch_order: Ordering,
        f: F,
    ) -> Result<Self::Value, Self::Value>
    where
        F: FnMut(Self::Value) -> Option<Self::Value>;
}

macro_rules! impl_atomic_fetch_update {
    ($atomic:ty, $value:ty) => {
        impl AtomicFetchUpdate for $atomic {
            type Value = $value;

            #[inline(always)]
            #[allow(deprecated)]
            fn fetch_update_compat<F>(
                &self,
                set_order: Ordering,
                fetch_order: Ordering,
                f: F,
            ) -> Result<$value, $value>
            where
                F: FnMut($value) -> Option<$value>,
            {
                self.fetch_update(set_order, fetch_order, f)
            }
        }
    };
}

impl_atomic_fetch_update!(AtomicU8, u8);
impl_atomic_fetch_update!(AtomicU16, u16);
impl_atomic_fetch_update!(AtomicU32, u32);
impl_atomic_fetch_update!(AtomicU64, u64);
impl_atomic_fetch_update!(AtomicUsize, usize);
