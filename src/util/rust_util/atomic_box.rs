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

impl<T> Drop for OnceOptionBox<T> {
    fn drop(&mut self) {
        let ptr = *self.inner.get_mut();
        if !ptr.is_null() {
            drop(unsafe { Box::from_raw(ptr) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct() {
        let oob = OnceOptionBox::<usize>::new();
        let elem = oob.get(Ordering::Relaxed);
        assert_eq!(elem, None);
    }

    #[test]
    fn init() {
        let oob = OnceOptionBox::<usize>::new();
        let elem = oob.get_or_init(Ordering::Relaxed, Ordering::Relaxed, || 42);
        assert_eq!(*elem, 42);
    }

    #[test]
    fn reinit() {
        let oob = OnceOptionBox::<usize>::new();
        let elem = oob.get_or_init(Ordering::Relaxed, Ordering::Relaxed, || 42);
        assert_eq!(*elem, 42);
        let elem2 = oob.get_or_init(Ordering::Relaxed, Ordering::Relaxed, || 43);
        assert_eq!(*elem2, 42);
    }
}
