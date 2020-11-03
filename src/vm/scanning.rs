use crate::plan::{Mutator, SelectedPlan, TransitiveClosure};
use crate::scheduler::gc_works::ProcessEdgesWork;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

/// VM-specific methods for scanning roots/objects.
pub trait Scanning<VM: VMBinding> {
    /// Scan stack roots after all mutators are paused.
    const SCAN_MUTATORS_IN_SAFEPOINT: bool = true;

    /// Scan all the mutators within a single work packet.
    ///
    /// `SCAN_MUTATORS_IN_SAFEPOINT` should also be enabled
    const SINGLE_THREAD_MUTATOR_SCANNING: bool = true;

    /// Delegated scanning of a object, processing each pointer field
    /// encountered. This method probably will be removed in the future,
    /// in favor of bulk scanning `scan_objects`.
    ///
    /// Arguments:
    /// * `trace`: The `TransitiveClosure` to use for scanning.
    /// * `object`: The object to be scanned.
    /// * `tls`: The GC worker thread that is doing this tracing.
    fn scan_object<T: TransitiveClosure>(
        trace: &mut T,
        object: ObjectReference,
        tls: OpaquePointer,
    );

    #[deprecated]
    fn reset_thread_counter();

    /// MMTk calls this method at the first time during a collection that thread's stacks
    /// have been scanned. This can be used (for example) to clean up
    /// obsolete compiled methods that are no longer being executed.
    ///
    /// Arguments:
    /// * `partial_scan`: Whether the scan was partial or full-heap.
    /// * `tls`: The GC thread that is performing the thread scan.
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: OpaquePointer);

    /// Bulk scanning of objects, processing each pointer field for each object.
    ///
    /// Arguments:
    /// * `objects`: The slice of object references to be scanned.
    fn scan_objects<W: ProcessEdgesWork<VM = VM>>(objects: &[ObjectReference]);

    /// Scan all the mutators for roots.
    fn scan_thread_roots<W: ProcessEdgesWork<VM = VM>>();

    /// Scan one mutator for roots.
    ///
    /// Arguments:
    /// * `mutator`: The reference to the mutator whose roots will be scanned.
    /// * `tls`: The GC thread that is performing this scanning.
    fn scan_thread_root<W: ProcessEdgesWork<VM = VM>>(
        mutator: &'static mut Mutator<SelectedPlan<VM>>,
        tls: OpaquePointer,
    );

    // TODO: compute_new_thread_roots

    /// Scan VM-specific roots. The creation of all root scan tasks (except thread scanning)
    /// goes here.
    fn scan_vm_specific_roots<W: ProcessEdgesWork<VM = VM>>();

    /// Return whether the VM supports return barriers. This is unused at the moment.
    fn supports_return_barrier() -> bool;
}
