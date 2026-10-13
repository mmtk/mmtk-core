//! Helpers for the `mock_test_concurrent_span_*` tests. Not a test file itself.

#![allow(dead_code)]

use super::mock_test_prelude::*;
use crate::plan::{AllocationSemantics, VectorObjectQueue};
use crate::policy::space::Space;
use crate::policy::spanspace::SpanSpace;
use crate::util::alloc::SpanAllocator;
use crate::util::{Address, ObjectReference};
use crate::{Mutator, MMTK};

/// The plan's `SpanSpace`, or `None` if the plan has none (the test is then skipped).
pub fn span_space(mmtk: &'static MMTK<MockVM>) -> Option<&'static SpanSpace<MockVM>> {
    let mut found: Option<*const SpanSpace<MockVM>> = None;
    mmtk.get_plan().for_each_space(&mut |s| {
        if let Some(ss) = s.downcast_ref::<SpanSpace<MockVM>>() {
            found = Some(ss as *const _);
        }
    });
    // Safety: the plan, and so its spaces, live as long as MMTk.
    found.map(|p| unsafe { &*p })
}

pub fn object_start(object: ObjectReference) -> Address {
    <MockVM as VMBinding>::VMObjectModel::ref_to_object_start(object)
}

/// Allocate an object of `size` bytes with the default semantics, and run `post_alloc`.
pub fn alloc_object(mutator: &mut Mutator<MockVM>, size: usize) -> ObjectReference {
    let addr = memory_manager::alloc(mutator, size, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
    let obj = MockVM::object_start_to_ref(addr);
    memory_manager::post_alloc(mutator, obj, size, AllocationSemantics::Default);
    obj
}

pub fn fill(obj: ObjectReference, size: usize, byte: u8) {
    unsafe { std::ptr::write_bytes(object_start(obj).to_mut_ptr::<u8>(), byte, size) };
}

pub fn check_filled(obj: ObjectReference, size: usize, byte: u8) {
    let start = object_start(obj);
    for i in 0..size {
        let b = unsafe { (start + i).load::<u8>() };
        assert_eq!(b, byte, "object {obj} byte {i} was overwritten");
    }
}

pub fn assert_zeroed(obj: ObjectReference, size: usize) {
    check_filled(obj, size, 0);
}

/// Object size of the span class `obj` was allocated in.
pub fn elem_size(space: &SpanSpace<MockVM>, obj: ObjectReference) -> usize {
    space.span_of(object_start(obj)).unwrap().elemsize()
}

/// Trace the given objects, as a GC worker would.
pub fn mark(space: &SpanSpace<MockVM>, objects: &[(ObjectReference, usize)]) {
    let mut queue = VectorObjectQueue::new();
    for (o, _) in objects {
        space.trace_object(&mut queue, *o);
    }
}

/// What the plan's mutator release does: return and sweep the cached spans.
pub fn release_span_allocator(mutator: &mut Mutator<MockVM>) {
    unsafe {
        mutator
            .allocator_impl_mut_for_semantic::<SpanAllocator<MockVM>>(AllocationSemantics::Default)
    }
    .release();
}

/// Sweep everything that is unswept, without allocating. Returns the pages swept.
pub fn sweep_all(space: &SpanSpace<MockVM>) -> usize {
    let mut pages = 0;
    loop {
        let swept = space.sweep_one();
        if swept == 0 {
            return pages;
        }
        pages += swept;
    }
}

/// A complete hand-driven cycle that marks `live` (see `mock_test_concurrent_span_alloc_sweep`).
pub fn cycle(
    space: &SpanSpace<MockVM>,
    mutator: &mut Mutator<MockVM>,
    live: &[(ObjectReference, usize)],
) {
    space.prepare_initial_mark();
    mark(space, live);
    space.release_after_mark();
    release_span_allocator(mutator);
    sweep_all(space);
}

pub fn reserved_data_pages(space: &SpanSpace<MockVM>) -> usize {
    space.get_page_resource().reserved_pages()
}
