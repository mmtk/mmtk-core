use crate::policy::space::Space;
use crate::vm::VMBinding;

use downcast_rs::Downcast;
use std::{ops::Deref, ops::DerefMut, sync::Arc};

/// `ArcFlexMut` is a replacement for `UnsafeCell` for a shared reference in situations where 1.
/// their mutability is managed by the programmer, 2. mutability is hard to reason about statically,
/// 3. using locks or `RefCell`/`AtomicRefCell` is not plausible for the sake of performance, and
/// 4. the shared reference could be both statically typed and a dyn ref to a trait object, depending
/// on where it is used.
/// `ArcFlexMut` does not guarantee thread safety, and it does not provide any actual locking.
/// It provides methods for acquiring a read or write guard, and can optionally check if there is
/// any possible data race. Without the checks, in a release build, `ArcFlexMut` should perform
/// as efficient as `UnsafeCell`.
/// We currently use this type for [`crate::policy::space::Space`]s.
#[repr(transparent)]
pub struct ArcFlexMut<T>
where
    T: ?Sized,
{
    inner: Arc<peace_lock::RwLock<T>>,
}

impl<T> ArcFlexMut<T> {
    /// Create a shared reference to the object.
    pub fn new(v: T) -> Self {
        Self {
            inner: Arc::new(peace_lock::RwLock::new(v)),
        }
    }
}

impl<T: ?Sized> ArcFlexMut<T> {
    /// Acquire a read guard to get immutable access to the data. It is allowed to have a reader when there is no writer.
    /// If the feature `check_flex_mut` is enabled, the method will panic if the rule is violated.
    pub fn read(&self) -> ArcFlexMutReadGuard<'_, T> {
        ArcFlexMutReadGuard {
            inner: self.inner.read(),
        }
    }

    /// Acquire a write guard to get mutable access to the data. It is allowed to have a writer when there is no other writer or reader.
    /// If the feature `check_flex_mut` is enabled, the method will panic if the rule is violated.
    pub fn write(&self) -> ArcFlexMutWriteGuard<'_, T> {
        ArcFlexMutWriteGuard {
            inner: self.inner.write(),
        }
    }
}

impl<T> Clone for ArcFlexMut<T>
where
    T: ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

// For types that implements `Downcast`, we can turn the shared reference into a reference of a concrete type.

impl<T: 'static + Downcast + ?Sized> ArcFlexMut<T> {
    /// Is it allowed to downcast to the given type?
    fn can_downcast<S: 'static>(&self) -> bool {
        let lock = self.inner.read();
        (*lock).as_any().downcast_ref::<S>().is_some()
    }

    /// Downcast the shared reference into a shared reference of a concrete type. The new reference share
    /// the count and the lock with the old consumed reference.
    pub fn downcast<S: 'static>(self) -> ArcFlexMut<S> {
        if self.can_downcast::<S>() {
            let raw = Arc::into_raw(self.inner);
            let new_inner = unsafe { Arc::from_raw(raw as *const peace_lock::RwLock<S>) };
            ArcFlexMut { inner: new_inner }
        } else {
            panic!("Failed to downcast")
        }
    }
}

// Methods to turn the shared reference into a shared reference of a trait object.
// The references points to the same object with the same count.
// This impl block is a workaround to implement the functionality specifically for
// `dyn Space`, as I can't find a way to implement this using generics.

macro_rules! to_trait_object {
    ($self: expr, $trait: ty) => {{
        let inner = $self.inner;
        let raw = Arc::into_raw(inner);
        let new_inner = unsafe { Arc::from_raw(raw as *const peace_lock::RwLock<$trait>) };
        ArcFlexMut { inner: new_inner }
    }};
}

impl<T> ArcFlexMut<T> {
    pub fn into_dyn_space<VM: VMBinding>(self) -> ArcFlexMut<dyn Space<VM>>
    where
        T: 'static + Space<VM>,
    {
        to_trait_object!(self, dyn Space<VM>)
    }
}

/// Read guard for ArcFlexMut
pub struct ArcFlexMutReadGuard<'a, T>
where
    T: ?Sized,
{
    inner: peace_lock::RwLockReadGuard<'a, T>,
}

impl<T> Deref for ArcFlexMutReadGuard<'_, T>
where
    T: ?Sized,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.inner.deref()
    }
}

/// Write guard for ArcFlexMut
pub struct ArcFlexMutWriteGuard<'a, T>
where
    T: ?Sized,
{
    inner: peace_lock::RwLockWriteGuard<'a, T>,
}

impl<T> Deref for ArcFlexMutWriteGuard<'_, T>
where
    T: ?Sized,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.inner.deref()
    }
}

impl<T> DerefMut for ArcFlexMutWriteGuard<'_, T>
where
    T: ?Sized,
{
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.inner.deref_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Foo(usize);
    trait Bar: 'static + Downcast {
        fn get(&self) -> usize;
        fn set(&mut self, v: usize);
    }
    impl Bar for Foo {
        fn get(&self) -> usize {
            self.0
        }
        fn set(&mut self, v: usize) {
            self.0 = v;
        }
    }

    impl<T> ArcFlexMut<T> {
        fn into_dyn_bar(self) -> ArcFlexMut<dyn Bar>
        where
            T: 'static + Bar,
        {
            to_trait_object!(self, dyn Bar)
        }
    }

    #[allow(clippy::redundant_clone)] // Allow redundant clone for testing the count
    #[test]
    fn create_clone_drop() {
        let r = ArcFlexMut::new(Foo(42));
        assert_eq!(Arc::strong_count(&r.inner), 1);

        {
            let r2 = r.clone();
            assert_eq!(r2.inner.read().get(), 42);
            assert_eq!(Arc::strong_count(&r2.inner), 2);
        }
        assert_eq!(Arc::strong_count(&r.inner), 1);
    }

    #[test]
    fn to_trait_object() {
        let r: ArcFlexMut<Foo> = ArcFlexMut::new(Foo(42));
        assert_eq!(Arc::strong_count(&r.inner), 1);

        let trait_obj: ArcFlexMut<dyn Bar> = r.clone().into_dyn_bar();
        assert_eq!(Arc::strong_count(&r.inner), 2);
        assert_eq!(r.inner.read().get(), 42);
        assert_eq!(Arc::strong_count(&trait_obj.inner), 2);
        assert_eq!(trait_obj.inner.read().get(), 42);

        drop(trait_obj);
        assert_eq!(Arc::strong_count(&r.inner), 1);
    }

    #[test]
    fn downcast() {
        let r = ArcFlexMut::new(Foo(42));
        let trait_obj: ArcFlexMut<dyn Bar> = r.into_dyn_bar();
        assert_eq!(Arc::strong_count(&trait_obj.inner), 1);

        let trait_obj_clone = trait_obj.clone();
        assert_eq!(Arc::strong_count(&trait_obj.inner), 2);

        let downcast: ArcFlexMut<Foo> = trait_obj_clone.downcast::<Foo>();
        assert_eq!(Arc::strong_count(&trait_obj.inner), 2);
        assert_eq!(Arc::strong_count(&downcast.inner), 2);
        assert_eq!(downcast.inner.read().get(), 42);
    }

    #[test]
    fn read() {
        let r = ArcFlexMut::new(Foo(42));
        assert_eq!(r.read().get(), 42);

        let read1 = r.read();
        let read2 = r.read();
        assert_eq!(read1.get(), 42);
        assert_eq!(read2.get(), 42);
    }

    #[allow(clippy::redundant_clone)] // Allow redundant clone for testing the count
    #[test]
    fn write() {
        let r = ArcFlexMut::new(Foo(42));
        let r2 = r.clone();
        let trait_obj = r.clone().into_dyn_bar();
        let downcast = trait_obj.clone().downcast::<Foo>();
        assert_eq!(Arc::strong_count(&r.inner), 4);

        r.write().set(1);
        assert_eq!(r.read().get(), 1);
        assert_eq!(r2.read().get(), 1);
        assert_eq!(trait_obj.read().get(), 1);
        assert_eq!(downcast.read().get(), 1);
    }

    #[test]
    fn multiple_readers() {
        let r = ArcFlexMut::new(Foo(42));
        let read1 = r.read();
        let read2 = r.read();
        assert_eq!(read1.get(), 42);
        assert_eq!(read2.get(), 42);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn multiple_writers() {
        let r = ArcFlexMut::new(Foo(42));
        let write1 = r.write();
        let write2 = r.write();
        assert_eq!(write1.get(), 42);
        assert_eq!(write2.get(), 42);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn mix_reader_writer() {
        let r = ArcFlexMut::new(Foo(42));
        let read = r.read();
        let write = r.write();
        assert_eq!(read.get(), 42);
        assert_eq!(write.get(), 42);
    }
}
