//! This module provides some convenient functions for scanning objects.

use crate::{
    util::{ObjectReference, VMWorkerThread},
    vm::{slot::Slot, ObjectTracer, Scanning, VMBinding},
};

/// Visit and potentially update the children of `object` using [`Scanning::scan_object`] or
/// [`Scanning::scan_object_and_trace_edges`] depending on the result of
/// [`Scanning::support_slot_enqueuing`].
///
/// This function is mainly used as a convenient function for simple node-enqueuing tracing loops.
pub fn visit_children<VM, const MAY_MOVE_OBJECTS: bool>(
    tls: VMWorkerThread,
    object: ObjectReference,
    object_tracer: &mut impl ObjectTracer,
) where
    VM: VMBinding,
{
    if VM::VMScanning::support_slot_enqueuing(tls, object) {
        VM::VMScanning::scan_object(tls, object, &mut |slot: <VM as VMBinding>::VMSlot| {
            if let Some(child) = slot.load() {
                let new_child = object_tracer.trace_object(child);
                if MAY_MOVE_OBJECTS {
                    if new_child != child {
                        slot.store(new_child);
                    }
                } else {
                    debug_assert_eq!(new_child, child);
                }
            }
        });
    } else {
        VM::VMScanning::scan_object_and_trace_edges(tls, object, &mut |child| {
            let new_child = object_tracer.trace_object(object);
            if !MAY_MOVE_OBJECTS {
                debug_assert_eq!(new_child, child);
            }
            new_child
        });
    }
}

/// Convenient wrapper of non-moving [`visit_children`].
pub fn visit_children_non_moving<VM>(
    tls: VMWorkerThread,
    object: ObjectReference,
    object_tracer: &mut impl ObjectTracer,
) where
    VM: VMBinding,
{
    visit_children::<VM, false>(tls, object, object_tracer)
}

/// Convenient wrapper of moving [`visit_children`].
pub fn visit_children_moving<VM>(
    tls: VMWorkerThread,
    object: ObjectReference,
    object_tracer: &mut impl ObjectTracer,
) where
    VM: VMBinding,
{
    visit_children::<VM, true>(tls, object, object_tracer)
}
