use crate::plan::Mutator;
use crate::util::VMWorkerThread;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

// Callback trait of scanning functions that report edges.
pub trait EdgeVisitor {
    /// Call this function for each edge.
    fn visit_edge(&mut self, edge: Address);
    // TODO: Add visit_soft_edge, visit_weak_edge, ... here.
}

/// Root-scanning methods use this trait to spawn work packets for processing roots.
///
/// Notes on the required traits:
///
/// -   `Clone`: The VM may divide one root-scanning call (such as `scan_vm_specific_roots`) into
///     multiple work packets to scan roots in parallel.  In this case, the factory shall be cloned
///     to be given to multiple work packets.
///
///     Cloning may be expensive if a factory contains many states. If the states are immutable, a
///     `RootsWorkFactory` implementation may hold those states in an `Arc` field so that multiple
///     factory instances can still share the part held in the `Arc` even after cloning.
///
/// -   `Send` + 'static: The factory will be given to root-scanning work packets.
///     Because work packets are distributed to and executed on different GC workers,
///     it needs `Send` to be sent between threads.  `'static` means it must not have
///     references to variables with limited lifetime (such as local variables), because
///     it needs to be moved between threads.
pub trait RootsWorkFactory: Clone + Send + 'static {
    /// Create work packets to handle the roots represented as edges.
    ///
    /// The work packet may update the edge.
    ///
    /// Arguments:
    /// * `edges`: A vector of edges.
    fn create_process_edge_roots_work(&mut self, edges: Vec<Address>);

    /// Create work packets to handle edges.
    ///
    /// The work packet cannot update the roots.  This is a good chance to pin the objects.
    ///
    /// This method is useful for conservative stack scanning, or VMs that cannot update some
    /// of the root edges.
    ///
    /// Arguments:
    /// * `nodes`: A vector of object references pointed by root edges.
    fn create_process_node_roots_work(&mut self, nodes: Vec<ObjectReference>);
}

/// VM-specific methods for scanning roots/objects.
pub trait Scanning<VM: VMBinding> {
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
    fn scan_thread_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory);

    /// Scan one mutator for roots.
    ///
    /// Arguments:
    /// * `mutator`: The reference to the mutator whose roots will be scanned.
    /// * `tls`: The GC thread that is performing this scanning.
    fn scan_thread_root(
        tls: VMWorkerThread,
        mutator: &'static mut Mutator<VM>,
        factory: impl RootsWorkFactory,
    );

    /// Scan VM-specific roots. The creation of all root scan tasks (except thread scanning)
    /// goes here.
    fn scan_vm_specific_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory);

    /// Return whether the VM supports return barriers. This is unused at the moment.
    fn supports_return_barrier() -> bool;

    fn prepare_for_roots_re_scanning();
}
