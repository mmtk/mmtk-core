use std::sync::Arc;

use crate::vm::VMBinding;
use crate::policy::space::Space;

pub use parking_lot_impl::SpaceRef;
pub use parking_lot_impl::downcast;
pub use parking_lot_impl::new;
// pub use peace_lock_impl::SpaceRef;
// pub use peace_lock_impl::downcast;
// pub use peace_lock_impl::new;

#[macro_export]
macro_rules! space_ref_write {
    ($r: expr) => {
        {
            trace!("{} acquire write lock on {}", std::panic::Location::caller(), stringify!($r));
            $r.write()
        }
    }
}

#[macro_export]
macro_rules! space_ref_read {
    ($r: expr) => {
        {
            trace!("{} acquire read lock on {}", std::panic::Location::caller(), stringify!($r));
            $r.read()
        }
    }
}

mod parking_lot_impl {
    use super::*;

    pub type SpaceRef<T> = Arc<parking_lot::RwLock<T>>;

    pub fn new<VM: VMBinding, S: Space<VM>>(s: S) -> SpaceRef<S> {
        Arc::new(parking_lot::RwLock::new(s))
    }

    pub fn downcast<VM: VMBinding, S: Space<VM>>(a: SpaceRef<dyn Space<VM>>) -> SpaceRef<S> {
        let lock = a.read();
        if lock.downcast_ref::<S>().is_some() {
            drop(lock);
            let raw = Arc::into_raw(a);
            unsafe { Arc::from_raw(raw as *const parking_lot::RwLock<S>) }
        } else {
            panic!("Failed to downcast")
        }
    }

}

mod peace_lock_impl {
    use super::*;

    pub type SpaceRef<T> = Arc<peace_lock::RwLock<T>>;

    pub fn new<VM: VMBinding, S: Space<VM>>(s: S) -> SpaceRef<S> {
        Arc::new(peace_lock::RwLock::new(s))
    }

    pub fn downcast<VM: VMBinding, S: Space<VM>>(a: SpaceRef<dyn Space<VM>>) -> SpaceRef<S> {
        let lock = a.read();
        if lock.downcast_ref::<S>().is_some() {
            drop(lock);
            let raw = Arc::into_raw(a);
            unsafe { Arc::from_raw(raw as *const peace_lock::RwLock<S>) }
        } else {
            panic!("Failed to downcast")
        }
    }
}
