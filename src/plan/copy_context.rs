use super::controller_collector_context::ControllerCollectorContext;
use super::MutatorContext;
use crate::mmtk::MMTK;
use crate::plan::transitive_closure::TransitiveClosure;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
#[cfg(feature = "sanity")]
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::constants::*;
use crate::util::conversions::bytes_to_pages;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::map::Map;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::options::{Options, UnsafeOptionsWrapper};
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::statistics::stats::Stats;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::plan::AllocationSemantics;
use enum_map::EnumMap;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A GC worker's context for copying GCs.
/// Each GC plan should provide their implementation of a CopyContext.
/// For non-copying GC, NoCopy can be used.
pub trait CopyContext: Sized + 'static + Sync + Send {
    type VM: VMBinding;
    const MAX_NON_LOS_COPY_BYTES: usize = MAX_INT;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self;
    fn init(&mut self, tls: OpaquePointer);
    fn prepare(&mut self);
    fn release(&mut self);
    fn alloc_copy(
        &mut self,
        original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        semantics: AllocationSemantics,
    ) -> Address;
    fn post_copy(
        &mut self,
        _obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _semantics: AllocationSemantics,
    ) {
    }
    fn copy_check_allocator(
        &self,
        _from: ObjectReference,
        bytes: usize,
        align: usize,
        semantics: AllocationSemantics,
    ) -> AllocationSemantics {
        let large = crate::util::alloc::allocator::get_maximum_aligned_size::<Self::VM>(
            bytes,
            align,
            Self::VM::MIN_ALIGNMENT,
        ) > Self::MAX_NON_LOS_COPY_BYTES;
        if large {
            AllocationSemantics::Los
        } else {
            semantics
        }
    }
}

pub struct Copier {
    
}
