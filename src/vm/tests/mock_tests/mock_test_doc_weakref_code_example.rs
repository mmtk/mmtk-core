//! This module tests the example code in `Scanning::process_weak_refs` and `weakref.md` in the
//! Porting Guide.  We only check if the example code compiles.  We cannot actually run it because
//! we can't construct a `GCWorker`.

use crate::{
    scheduler::GCWorker,
    util::ObjectReference,
    vm::{ObjectTracer, ObjectTracerContext, Scanning, VMBinding},
};

use super::mock_test_prelude::MockVM;

#[allow(dead_code)] // We don't construct this struct as we can't run it.
struct VMScanning;

// Just to make the code example look better.
use MockVM as MyVM;

// Placeholders for functions supposed to be implemented by the VM.
#[allow(dead_code)]
mod my_vm {
    use crate::util::ObjectReference;

    pub fn get_finalizable_object() -> Vec<ObjectReference> {
        unimplemented!()
    }

    pub fn set_new_finalizable_objects(_objects: Vec<ObjectReference>) {}

    pub fn enqueue_finalizable_object_to_be_executed_later(_object: ObjectReference) {}
}

// ANCHOR: process_weak_refs_finalization
impl Scanning<MyVM> for VMScanning {
    fn process_weak_refs(
        worker: &mut GCWorker<MyVM>,
        tracer_context: impl ObjectTracerContext<MyVM>,
    ) -> bool {
        let finalizable_objects: Vec<ObjectReference> = my_vm::get_finalizable_object();
        let mut new_finalizable_objects = vec![];

        tracer_context.with_tracer(worker, |tracer| {
            for object in finalizable_objects {
                if object.is_reachable() {
                    // `object` is still reachable.
                    // It may have been moved if it is a copying GC.
                    let new_object = object.get_forwarded_object().unwrap_or(object);
                    new_finalizable_objects.push(new_object);
                } else {
                    // `object` is unreachable.
                    // Retain it, and enqueue it for postponed finalization.
                    let new_object = tracer.trace_object(object);
                    my_vm::enqueue_finalizable_object_to_be_executed_later(new_object);
                }
            }
        });

        my_vm::set_new_finalizable_objects(new_finalizable_objects);

        false
    }

    // ...
    // ANCHOR_END: process_weak_refs_finalization

    // Methods after this are placeholders.  We only ensure they compile.

    fn scan_object<SV: crate::vm::SlotVisitor<<MockVM as VMBinding>::VMSlot>>(
        _tls: crate::util::VMWorkerThread,
        _object: ObjectReference,
        _slot_visitor: &mut SV,
    ) {
        unimplemented!()
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: crate::util::VMWorkerThread) {
        unimplemented!()
    }

    fn scan_roots_in_mutator_thread(
        _tls: crate::util::VMWorkerThread,
        _mutator: &'static mut crate::Mutator<MockVM>,
        _factory: impl crate::vm::RootsWorkFactory<<MockVM as VMBinding>::VMSlot>,
    ) {
        unimplemented!()
    }

    fn scan_vm_specific_roots(
        _tls: crate::util::VMWorkerThread,
        _factory: impl crate::vm::RootsWorkFactory<<MockVM as VMBinding>::VMSlot>,
    ) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }

    fn prepare_for_roots_re_scanning() {
        unimplemented!()
    }
}
