use crate::plan::Mutator;
use crate::scheduler::ProcessEdgesWork;
use crate::util::VMWorkerThread;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

// Callback trait of scanning functions that report edges.
pub trait EdgeVisitor {
    /// Call this function for each edge.
    fn visit_edge(&mut self, edge: Address);
    // TODO: Add visit_soft_edge, visit_weak_edge, ... here.
}

/// VM-specific methods for scanning roots/objects.
pub trait Scanning<VM: VMBinding> {
    const SCAN_THREAD_STACK_CONSERVATIVELY: bool = VM::CONSERVATIVE_SCANNING;
    /// Scan stack roots after all mutators are paused.
    const SCAN_MUTATORS_IN_SAFEPOINT: bool = true;

    /// Scan all the mutators within a single work packet.
    ///
    /// `SCAN_MUTATORS_IN_SAFEPOINT` should also be enabled
    const SINGLE_THREAD_MUTATOR_SCANNING: bool = true;

    /// Delegated scanning of a object, visiting each pointer field
    /// encountered.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `edge_visitor`: Called back for each edge.
    fn scan_object<EV: EdgeVisitor>(
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut EV,
    );

    /// MMTk calls this method at the first time during a collection that thread's stacks
    /// have been scanned. This can be used (for example) to clean up
    /// obsolete compiled methods that are no longer being executed.
    ///
    /// Arguments:
    /// * `partial_scan`: Whether the scan was partial or full-heap.
    /// * `tls`: The GC thread that is performing the thread scan.
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: VMWorkerThread);

    /// Bulk scanning of objects, processing each pointer field for each object.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `objects`: The slice of object references to be scanned.
    /// * `edge_visitor`: Called back for each edge in each object in `objects`.
    fn scan_objects<EV: EdgeVisitor>(
        tls: VMWorkerThread,
        objects: &[ObjectReference],
        edge_visitor: &mut EV,
    ) {
        for object in objects.iter() {
            Self::scan_object(tls, *object, edge_visitor);
        }
    }

    /// Scan all the mutators for roots.
    fn scan_thread_roots<W: ProcessEdgesWork<VM = VM>>();

    /// Scan one mutator for roots.
    ///
    /// Arguments:
    /// * `mutator`: The reference to the mutator whose roots will be scanned.
    /// * `tls`: The GC thread that is performing this scanning.
    fn scan_thread_root<W: ProcessEdgesWork<VM = VM>>(
        mutator: &'static mut Mutator<VM>,
        tls: VMWorkerThread,
    );

    /// Scan VM-specific roots. The creation of all root scan tasks (except thread scanning)
    /// goes here.
    fn scan_vm_specific_roots<W: ProcessEdgesWork<VM = VM>>();

    /// Return whether the VM supports return barriers. This is unused at the moment.
    fn supports_return_barrier() -> bool;

    fn prepare_for_roots_re_scanning();
}
