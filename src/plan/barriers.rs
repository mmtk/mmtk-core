//! Read/Write barrier implementations.

use crate::util::*;
use downcast_rs::Downcast;

/// BarrierSelector describes which barrier to use.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BarrierSelector {
    NoBarrier,
    ObjectBarrier,
}

impl BarrierSelector {
    pub const fn equals(&self, other: BarrierSelector) -> bool {
        // cast enum to u8 then compare. Otherwise, we cannot do it in a const fn.
        *self as u8 == other as u8
    }
}

pub trait Barrier: 'static + Send + Downcast {
    fn flush(&mut self) {}

    /// Subsuming barrier for object reference write
    fn object_reference_write(
        &mut self,
        src: ObjectReference,
        slot: Address,
        target: ObjectReference,
    ) {
        self.object_reference_write_pre(src, slot, target);
        unsafe { slot.store(target) };
        self.object_reference_write_post(src, slot, target);
    }

    /// Full pre-barrier for object reference write
    fn object_reference_write_pre(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    /// Full post-barrier for object reference write
    fn object_reference_write_post(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    /// Object reference write slow-path call.
    /// This can be called either before or after the store, depend on the concrete barrier implementation.
    fn object_reference_write_slow(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    /// Subsuming barrier for array copy
    fn array_copy(&mut self, src: Address, dst: Address, count: usize) {
        self.array_copy_pre(src, dst, count);
        unsafe { std::ptr::copy::<ObjectReference>(src.to_ptr(), dst.to_mut_ptr(), count) };
        self.array_copy_post(src, dst, count);
    }

    /// Full pre-barrier for array copy
    fn array_copy_pre(&mut self, _src: Address, _dst: Address, _count: usize) {}

    /// Full post-barrier for array copy
    fn array_copy_post(&mut self, _src: Address, _dst: Address, _count: usize) {}
}

impl_downcast!(Barrier);

/// Empty barrier implementation.
/// For GCs that do not need any barriers
pub struct NoBarrier;

impl Barrier for NoBarrier {}

pub use super::generational::barrier::GenObjectBarrier;
