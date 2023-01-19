//! This module works around limitations of the Rust programming language, and provides missing
//! functionalities that we may expect the Rust programming language and its standard libraries
//! to provide.

/// Const function for min value of two usize numbers.
pub const fn min_of_usize(a: usize, b: usize) -> usize {
    if a > b {
        b
    } else {
        a
    }
}

#[cfg(feature = "nightly")]
use core::intrinsics::{likely, unlikely};

// likely() and unlikely() compiler hints in stable Rust
// [1]: https://github.com/rust-lang/hashbrown/blob/a41bd76de0a53838725b997c6085e024c47a0455/src/raw/mod.rs#L48-L70
// [2]: https://users.rust-lang.org/t/compiler-hint-for-unlikely-likely-for-if-branches/62102/3
#[cfg(not(feature = "nightly"))]
#[inline]
#[cold]
fn cold() {}

#[cfg(not(feature = "nightly"))]
#[inline]
pub fn likely(b: bool) -> bool {
    if !b {
        cold();
    }
    b
}
#[cfg(not(feature = "nightly"))]
#[inline]
pub fn unlikely(b: bool) -> bool {
    if b {
        cold();
    }
    b
}

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Once;

/// InitializeOnce creates an uninitialized value that needs to be manually initialized later. InitializeOnce
/// guarantees the value is only initialized once. This type is used to allow more efficient reads.
/// Unlike the `lazy_static!` which checks whether the static is initialized
/// in every read, InitializeOnce has no extra check for reads.
pub struct InitializeOnce<T: 'static> {
    v: UnsafeCell<MaybeUninit<T>>,
    /// This is used to guarantee `init_fn` is only called once.
    once: Once,
}

impl<T> InitializeOnce<T> {
    pub const fn new() -> Self {
        InitializeOnce {
            v: UnsafeCell::new(MaybeUninit::uninit()),
            once: Once::new(),
        }
    }

    /// Initialize the value. This should be called before ever using the struct.
    /// If this method is called by multiple threads, the first thread will
    /// initialize the value, and the other threads will be blocked until the
    /// initialization is done (`Once` returns).
    pub fn initialize_once(&self, init_fn: &'static dyn Fn() -> T) {
        self.once.call_once(|| {
            unsafe { &mut *self.v.get() }.write(init_fn());
        });
        debug_assert!(self.once.is_completed());
    }

    /// Get the value. This should only be used after initialize_once()
    #[inline(always)]
    pub fn get_ref(&self) -> &T {
        // We only assert in debug builds.
        debug_assert!(self.once.is_completed());
        unsafe { (*self.v.get()).assume_init_ref() }
    }
}

impl<T> std::ops::Deref for InitializeOnce<T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.get_ref()
    }
}

unsafe impl<T> Sync for InitializeOnce<T> {}

#[cfg(test)]
mod initialize_once_tests {
    use super::*;

    #[test]
    fn test_threads_compete_initialize() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;
        use std::thread;

        // Create multiple threads to initialize the same `InitializeOnce` value
        const N_THREADS: usize = 1000;
        // The test value
        static I: InitializeOnce<usize> = InitializeOnce::new();
        // Count how many times the function is called
        static INITIALIZE_COUNT: AtomicUsize = AtomicUsize::new(0);
        // The function to create initial value
        fn initialize_usize() -> usize {
            INITIALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
            42
        }

        let mut threads = vec![];
        for _ in 1..N_THREADS {
            threads.push(thread::spawn(|| {
                I.initialize_once(&initialize_usize);
                // Every thread should see the value correctly initialized.
                assert_eq!(*I, 42);
            }));
        }
        threads.into_iter().for_each(|t| t.join().unwrap());

        // The initialize_usize should only be called once
        assert_eq!(INITIALIZE_COUNT.load(Ordering::SeqCst), 1);
    }
}

/// This module is for allocating large arrays or vectors with initial zero values.
///
/// Note: The standard library uses the `IsZero` trait to specialize the intialization of `Vec<T>`
/// if the initial element values are zero.  Primitive type, such as `i8`, `usize`, `f32`, as well
/// as types with known representations such as `Option<NonZeroUsize>` implement the `IsZero`
/// trait.  However, it has several limitations.
///
/// 1.  Composite types, such as `SpaceDescriptor(usize)`, doesn't implement the `IsZero` trait,
///     even if it has the `#[repr(transparent)]` annotation.
/// 2.  The `IsZero` trait is private to the `std` module, and we cannot use it.
///
/// Therefore, `vec![0usize; 33554432]` takes only 4 **microseconds**, while
/// `vec![SpaceDescriptor(0); 33554432]` will take 22 **milliseconds** to execute on some machine.
/// If such an allocation happens during start-up, the delay will be noticeable to light-weight
/// scripting languages, such as Ruby.
///
/// We implement our own fast allocation of large zeroed vectors in this module.  If one day Rust
/// provides a standard way to optimize for zeroed allocation of vectors of composite types, we
/// can switch to the standard mechanism.
pub mod zeroed_alloc {

    use std::alloc::{alloc_zeroed, Layout};

    /// Allocate a `Vec<T>` of all-zero values.
    ///
    /// This intends to be a faster alternative to `vec![T(0), size]`.  It will allocate pre-zeroed
    /// buffer, and not store zero values to its elements as part of initialization.
    ///
    /// It is useful when creating large (hundreds of megabytes) Vecs when the execution time is
    /// critical (such as during start-up, where a 100ms delay is obvious to small applications.)
    /// However, because of its unsafe nature, it should only be used when necessary.
    ///
    /// Arguments:
    ///
    /// -   `T`: The element type.
    /// -   `size`: The length and capacity of the created vector.
    ///
    /// Returns the created vector.
    ///
    /// # Unsafe
    ///
    /// This function is unsafe.  It will not call any constructor of `T`.  The user must ensure
    /// that a value with all bits being zero is meaningful for type `T`.
    pub(crate) unsafe fn new_zeroed_vec<T>(size: usize) -> Vec<T> {
        let layout = Layout::array::<T>(size).unwrap();
        let ptr = alloc_zeroed(layout) as *mut T;
        Vec::from_raw_parts(ptr, size, size)
    }
}
