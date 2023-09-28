use crate::vm::VMBinding;
use crate::policy::space::Space;

use std::{sync::Arc, ops::Deref, ops::DerefMut};
use downcast_rs::Downcast;

macro_rules! to_trait_object {
    ($self: expr, $trait: ty) => {
        {
            let inner = $self.inner;
            let raw = Arc::into_raw(inner);
            let new_inner = unsafe { Arc::from_raw(raw as *const peace_lock::RwLock<$trait>) };
            SharedRef { inner: new_inner }
        }
    }
}

pub struct SharedRef<T> where T: ?Sized {
    inner: Arc<peace_lock::RwLock<T>>,
}

impl<T> SharedRef<T> {
    pub fn new(v: T) -> Self {
        Self {
            inner: Arc::new(peace_lock::RwLock::new(v)),
        }
    }
}

impl<T> SharedRef<T> {
    pub fn to_dyn_space<VM: VMBinding>(self) -> SharedRef<dyn Space<VM>> where T: 'static + Space<VM> {
        to_trait_object!(self, dyn Space<VM>)
    }
}

impl<T: ?Sized> SharedRef<T> {
    pub fn read<'a>(&'a self) -> SharedRefReadGuard<'a, T> {
        SharedRefReadGuard { inner: self.inner.read() }
    }

    pub fn write<'a>(&'a self) -> SharedRefWriteGuard<'a, T> {
        SharedRefWriteGuard { inner: self.inner.write() }
    }
}

impl<T: 'static + Downcast + ?Sized> SharedRef<T> {
    fn can_downcast<S: 'static>(&self) -> bool {
        let lock = self.inner.read();
        (&*lock).as_any().downcast_ref::<S>().is_some()
    }

    pub fn downcast<S: 'static>(self) -> SharedRef<S> {
        if self.can_downcast::<S>() {
            let raw = Arc::into_raw(self.inner);
            let new_inner = unsafe { Arc::from_raw(raw as *const peace_lock::RwLock<S>) };
            SharedRef {
                inner: new_inner,
            }
        } else {
            panic!("Failed to downcast")
        }
    }
}

impl<T> Clone for SharedRef<T> where T: ?Sized {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone()
        }
    }
}

pub struct SharedRefReadGuard<'a, T> where T: ?Sized {
    inner: peace_lock::RwLockReadGuard<'a, T>,
}

impl<T> Deref for SharedRefReadGuard<'_, T> where T: ?Sized {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.inner.deref()
    }
}

pub struct SharedRefWriteGuard<'a, T> where T: ?Sized {
    inner: peace_lock::RwLockWriteGuard<'a, T>,
}

impl<T> Deref for SharedRefWriteGuard<'_, T> where T: ?Sized {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.inner.deref()
    }
}

impl<T> DerefMut for SharedRefWriteGuard<'_, T> where T: ?Sized {
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

    impl<T> SharedRef<T> {
        fn to_dyn_bar(self) -> SharedRef<dyn Bar> where T: 'static + Bar {
            to_trait_object!(self, dyn Bar)
        }
    }

    #[test]
    fn create_clone_drop() {
        let r = SharedRef::new(Foo(42));
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
        let r: SharedRef<Foo> = SharedRef::new(Foo(42));
        assert_eq!(Arc::strong_count(&r.inner), 1);

        let trait_obj: SharedRef<dyn Bar> = r.clone().to_dyn_bar();
        assert_eq!(Arc::strong_count(&r.inner), 2);
        assert_eq!(r.inner.read().get(), 42);
        assert_eq!(Arc::strong_count(&trait_obj.inner), 2);
        assert_eq!(trait_obj.inner.read().get(), 42);

        drop(trait_obj);
        assert_eq!(Arc::strong_count(&r.inner), 1);
    }

    #[test]
    fn downcast() {
        let r = SharedRef::new(Foo(42));
        let trait_obj: SharedRef<dyn Bar> = r.to_dyn_bar();
        assert_eq!(Arc::strong_count(&trait_obj.inner), 1);

        let trait_obj_clone = trait_obj.clone();
        assert_eq!(Arc::strong_count(&trait_obj.inner), 2);

        let downcast: SharedRef<Foo> = trait_obj_clone.downcast::<Foo>();
        assert_eq!(Arc::strong_count(&trait_obj.inner), 2);
        assert_eq!(Arc::strong_count(&downcast.inner), 2);
        assert_eq!(downcast.inner.read().get(), 42);
    }

    #[test]
    fn read() {
        let r = SharedRef::new(Foo(42));
        assert_eq!(r.read().get(), 42);

        let read1 = r.read();
        let read2 = r.read();
        assert_eq!(read1.get(), 42);
        assert_eq!(read2.get(), 42);
    }

    #[test]
    fn write() {
        let r = SharedRef::new(Foo(42));
        let r2 = r.clone();
        let trait_obj = r.clone().to_dyn_bar();
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
        let r = SharedRef::new(Foo(42));
        let read1 = r.read();
        let read2 = r.read();
        assert_eq!(read1.get(), 42);
        assert_eq!(read2.get(), 42);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn multiple_writers() {
        let r = SharedRef::new(Foo(42));
        let write1 = r.write();
        let write2 = r.write();
        assert_eq!(write1.get(), 42);
        assert_eq!(write2.get(), 42);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn mix_reader_writer() {
        let r = SharedRef::new(Foo(42));
        let read = r.read();
        let write = r.write();
        assert_eq!(read.get(), 42);
        assert_eq!(write.get(), 42);
    }
}