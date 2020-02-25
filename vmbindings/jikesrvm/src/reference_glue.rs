use libc::c_void;

use mmtk::vm::ReferenceGlue;
use mmtk::util::{Address, ObjectReference};
use mmtk::TraceLocal;
use mmtk::util::reference_processor::*;
use mmtk::util::OpaquePointer;

use entrypoint::*;
use JikesRVM;

pub struct VMReferenceGlue {}

impl ReferenceGlue<JikesRVM> for VMReferenceGlue {
    fn set_referent(reff: ObjectReference, referent: ObjectReference) {
        unsafe {
            (reff.to_address() + REFERENCE_REFERENT_FIELD_OFFSET).store(referent.value());
        }
    }

    fn get_referent(object: ObjectReference) -> ObjectReference {
        debug_assert!(!object.is_null());
        unsafe {
            (object.to_address() + REFERENCE_REFERENT_FIELD_OFFSET).load::<ObjectReference>()
        }
    }

    /**
     * Processes a reference with the current semantics.
     * <p>
     * This method deals with a soft reference as if it were a weak reference, i.e.
     * it does not retain the referent. To retain the referent, use
     * {@link #retainReferent(TraceLocal, ObjectReference)} followed by a transitive
     * closure phase.
     *
     * @param reference the address of the reference. This may or may not
     * be the address of a heap object, depending on the VM.
     * @param trace the thread local trace element.
     * @return an updated reference (e.g. with a new address) if the reference
     *  is still live, {@code ObjectReference.nullReference()} otherwise
     */
    fn process_reference<T: TraceLocal>(trace: &mut T, reference: ObjectReference, tls: OpaquePointer) -> ObjectReference {
        debug_assert!(!reference.is_null());

        if TRACE_DETAIL { trace!("Processing reference: {:?}", reference); }

        /*
         * If the reference is dead, we're done with it. Let it (and
         * possibly its referent) be garbage-collected.
         */
        if !trace.is_live(reference) {
            VMReferenceGlue::clear_referent(reference);                   // Too much paranoia ...
            if TRACE_UNREACHABLE { trace!(" UNREACHABLE reference: {:?}", reference); }
            if TRACE_DETAIL { trace!(" (unreachable)"); }
            return unsafe { Address::zero().to_object_reference() };
        }

        /* The reference object is live */
        let new_reference = trace.get_forwarded_reference(reference);
        let old_referent = VMReferenceGlue::get_referent(reference);

        if TRACE_DETAIL { trace!(" ~> {:?}", old_referent); }

        /*
         * If the application has cleared the referent the Java spec says
         * this does not cause the Reference object to be enqueued. We
         * simply allow the Reference object to fall out of our
         * waiting list.
         */
        if old_referent.is_null() {
            if TRACE_DETAIL { trace!("(null referent)"); }
            return unsafe { Address::zero().to_object_reference() };
        }

        if TRACE_DETAIL { trace!(" => {:?}", new_reference); }

        if trace.is_live(old_referent) {
            if cfg!(feature = "debug") {
                // FIXME
                /*if (!DebugUtil.validRef(oldReferent)) {
                    VM.sysWriteln("Error in old referent.");
                    DebugUtil.dumpRef(oldReferent);
                    VM.sysFail("Invalid reference");
                }*/
            }

            /*
             * Referent is still reachable in a way that is as strong as
             * or stronger than the current reference level.
             */
            let new_referent = trace.get_forwarded_referent(old_referent);

            if TRACE_DETAIL { trace!(" ~> {:?}", new_referent); }

            if cfg!(feature = "debug") {
                // FIXME
                /*if (!DebugUtil.validRef(newReferent)) {
                    VM.sysWriteln("Error forwarding reference object.");
                    DebugUtil.dumpRef(oldReferent);
                    VM.sysFail("Invalid reference");
                }*/
                debug_assert!(trace.is_live(new_reference));
            }

            /*
             * The reference object stays on the waiting list, and the
             * referent is untouched. The only thing we must do is
             * ensure that the former addresses are updated with the
             * new forwarding addresses in case the collector is a
             * copying collector.
             */

            /* Update the referent */
            VMReferenceGlue::set_referent(new_reference, new_referent);
            return new_reference;
        } else {
            /* Referent is unreachable. Clear the referent and enqueue the reference object. */

            if TRACE_DETAIL { trace!(" UNREACHABLE"); }
                else if TRACE_UNREACHABLE { trace!(" UNREACHABLE referent: {:?}", old_referent); }

            VMReferenceGlue::clear_referent(new_reference);
            let new_reference_raw = new_reference.value() as *mut c_void;
            unsafe { jtoc_call!(ENQUEUE_REFERENCE_METHOD_OFFSET, tls, new_reference_raw); }
            return unsafe { Address::zero().to_object_reference() };
        }
    }
}