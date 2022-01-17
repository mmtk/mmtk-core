/// Const funciton for min value of two usize numbers.
pub const fn min_of_usize(a: usize, b: usize) -> usize {
    if a > b {
        b
    } else {
        a
    }
}

use std::mem::MaybeUninit;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// InitializeOnce creates an uninitialized value that needs to be initialized later. InitializeOnce
/// guarantees the value is only initialized once. This type is used to allow more efficient reads.
/// Unlike the `lazy_static!` which checks whether the static is initialized
/// in every read, InitializeOnce has no extra check for reads.
pub struct InitializeOnce<T> {
    v: MaybeUninit<T>,
    initialized: AtomicBool,
}

impl<T> InitializeOnce<T> {
    pub const fn new() -> Self {
        InitializeOnce {
            v: MaybeUninit::uninit(),
            initialized: AtomicBool::new(false),
        }
    }

    /// Initialize the value. This should be called before ever using the struct.
    pub fn initialize(&self, val: T) {
        if self
            .initialized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // This is only called once.
            let mut_self: &mut InitializeOnce<T> = unsafe { &mut *(self as *const _ as *mut _) };
            mut_self.v.write(val);
        }
        debug_assert!(self.initialized.load(Ordering::SeqCst));
    }

    /// Get the value. This should only be used after initialize()
    #[inline(always)]
    pub fn get_ref(&self) -> &T {
        // We only assert in debug builds.
        debug_assert!(self.initialized.load(Ordering::SeqCst));
        unsafe { self.v.assume_init_ref() }
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
