use std::sync::atomic::{AtomicPtr, Ordering};

/// A lazily initialized box.  Similar to an `Option<Box<T>>`, but can be initialized atomically.
pub struct OnceOptionBox<T> {
    inner: AtomicPtr<T>,
}

impl<T> OnceOptionBox<T> {
    pub fn new() -> OnceOptionBox<T> {
        Self {
            inner: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    pub fn get(&self, order: Ordering) -> Option<&T> {
        let ptr = self.inner.load(order);
        unsafe { ptr.as_ref() }
    }

    pub fn get_or_init(
        &self,
        order_load: Ordering,
        order_store: Ordering,
        init: impl FnOnce() -> T,
    ) -> &T {
        if let Some(get_result) = self.get(order_load) {
            return get_result;
        }

        let new_inner = Box::into_raw(Box::new(init()));
        let cas_result = self.inner.compare_exchange(
            std::ptr::null_mut(),
            new_inner,
            order_store,
            Ordering::Relaxed,
        );
        match cas_result {
            Ok(old_inner) => {
                debug_assert_eq!(old_inner, std::ptr::null_mut());
                unsafe { new_inner.as_ref().unwrap() }
            }
            Err(old_inner) => {
                drop(unsafe { Box::from_raw(new_inner) });
                unsafe { old_inner.as_ref().unwrap() }
            }
        }
    }
}
