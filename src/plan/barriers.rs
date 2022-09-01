//! Read/Write barrier implementations.

use crate::util::metadata::{compare_exchange_metadata, load_metadata};
use crate::vm::ObjectModel;
use crate::{
    util::{metadata::MetadataSpec, *},
    vm::VMBinding,
};
use atomic::Ordering;
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

pub trait BarrierSemantics: 'static + Send {
    type VM: VMBinding;

    const UNLOG_BIT_SPEC: MetadataSpec =
        *<Self::VM as VMBinding>::VMObjectModel::GLOBAL_LOG_BIT_SPEC.as_spec();

    fn flush(&mut self);

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        slot: Address,
        target: ObjectReference,
    );

    fn array_copy_slow(&mut self, src: Address, dst: Address, count: usize);
}

pub struct ObjectBarrier<S: BarrierSemantics> {
    semantics: S,
}

impl<S: BarrierSemantics> ObjectBarrier<S> {
    pub fn new(semantics: S) -> Self {
        Self { semantics }
    }

    /// Attepmt to atomically log an object.
    /// Returns true if the object is not logged previously.
    #[inline(always)]
    fn object_is_unlogged(&self, object: ObjectReference) -> bool {
        load_metadata::<S::VM>(&S::UNLOG_BIT_SPEC, object, None, None) != 0
    }

    /// Attepmt to atomically log an object.
    /// Returns true if the object is not logged previously.
    #[inline(always)]
    fn log_object(&self, object: ObjectReference) -> bool {
        loop {
            let old_value =
                load_metadata::<S::VM>(&S::UNLOG_BIT_SPEC, object, None, Some(Ordering::SeqCst));
            if old_value == 0 {
                return false;
            }
            if compare_exchange_metadata::<S::VM>(
                &S::UNLOG_BIT_SPEC,
                object,
                1,
                0,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                return true;
            }
        }
    }
}

impl<S: BarrierSemantics> Barrier for ObjectBarrier<S> {
    fn flush(&mut self) {
        self.semantics.flush();
    }

    #[inline(always)]
    fn object_reference_write_post(
        &mut self,
        src: ObjectReference,
        slot: Address,
        target: ObjectReference,
    ) {
        if self.object_is_unlogged(src) {
            self.object_reference_write_slow(src, slot, target);
        }
    }

    #[inline(always)]
    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        slot: Address,
        target: ObjectReference,
    ) {
        if self.log_object(src) {
            self.semantics
                .object_reference_write_slow(src, slot, target);
        }
    }

    #[inline(always)]
    fn array_copy_post(&mut self, src: Address, dst: Address, count: usize) {
        debug_assert!(!dst.is_zero());
        self.semantics.array_copy_slow(src, dst, count);
    }
}
