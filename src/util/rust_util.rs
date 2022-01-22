/// Const funciton for min value of two usize numbers.
pub const fn min_of_usize(a: usize, b: usize) -> usize {
    if a > b {
        b
    } else {
        a
    }
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
    /// The function that is used to create the initialization value. This will be only called once.
    init_fn: &'static dyn Fn() -> T,
    /// This is used to guarantee `init_fn` is only called once.
    once: Once,
}

impl<T> InitializeOnce<T> {
    pub const fn new(init_fn: &'static dyn Fn() -> T) -> Self {
        InitializeOnce {
            v: UnsafeCell::new(MaybeUninit::uninit()),
            init_fn,
            once: Once::new(),
        }
    }

    /// Initialize the value. This should be called before ever using the struct.
    /// If this method is called by multiple threads, the first thread will
    /// initialize the value, and the other threads will be blocked until the
    /// initialization is done (`Once` returns).
    pub fn initialize_once(&self) {
        self.once.call_once(|| {
            unsafe { &mut *self.v.get() }.write((self.init_fn)());
        });
        debug_assert!(self.once.is_completed());
    }

    /// Get the value. This should only be used after initialize_once()
    #[inline(always)]
    pub fn get_ref(&self) -> &T {
        // We only assert in debug builds.
        debug_assert!(self.once.is_completed());
        unsafe { (&*self.v.get()).assume_init_ref() }
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
        static I: InitializeOnce<usize> = InitializeOnce::new(&initialize_usize);
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
                I.initialize_once();
                // Every thread should see the value correctly initialized.
                assert_eq!(*I, 42);
            }));
        }
        threads.into_iter().for_each(|t| t.join().unwrap());

        // The initialize_usize should only be called once
        assert_eq!(INITIALIZE_COUNT.load(Ordering::SeqCst), 1);
    }
}
