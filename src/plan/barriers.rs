//! Read/Write barrier implementations.

use crate::util::*;

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

pub trait Barrier: 'static + Send {
    fn flush(&mut self) {}

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

    fn object_reference_write_pre(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    fn object_reference_write_post(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    fn array_copy(
        &mut self,
        src_object: Option<ObjectReference>,
        src: Address,
        dst_object: Option<ObjectReference>,
        dst: Address,
        count: usize,
    ) {
        self.array_copy_pre(src_object, src, dst_object, dst, count);
        unsafe { std::ptr::copy::<ObjectReference>(src.to_ptr(), dst.to_mut_ptr(), count) };
        self.array_copy_post(src_object, src, dst_object, dst, count);
    }

    fn array_copy_pre(
        &mut self,
        _src_object: Option<ObjectReference>,
        _src: Address,
        _dst_object: Option<ObjectReference>,
        _dst: Address,
        _count: usize,
    ) {
    }

    fn array_copy_post(
        &mut self,
        _src_object: Option<ObjectReference>,
        _src: Address,
        _dst_object: Option<ObjectReference>,
        _dst: Address,
        _count: usize,
    ) {
    }
}

pub struct NoBarrier;

impl Barrier for NoBarrier {}

pub use super::generational::barrier::GenObjectBarrier;
