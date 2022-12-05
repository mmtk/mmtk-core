use crate::plan::Mutator;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::edge_shape::Edge;
use crate::vm::VMBinding;

/// Callback trait of scanning functions that report edges.
pub trait EdgeVisitor<ES: Edge> {
    /// Call this function for each edge.
    fn visit_edge(&mut self, edge: ES);
}

/// This lets us use closures as EdgeVisitor.
impl<ES: Edge, F: FnMut(ES)> EdgeVisitor<ES> for F {
    fn visit_edge(&mut self, edge: ES) {
        self(edge)
    }
}

/// Callback trait of scanning functions that directly trace through edges.
pub trait ObjectTracer {
    /// Call this function for the content of each edge,
    /// and assign the returned value back to the edge.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;
}

/// This lets us use closures as ObjectTracer.
impl<F: FnMut(ObjectReference) -> ObjectReference> ObjectTracer for F {
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        self(object)
    }
}

/// Root-scanning methods use this trait to create work packets for processing roots.
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
pub trait RootsWorkFactory<ES: Edge>: Clone + Send + 'static {
    /// Create work packets to handle root edges.
    ///
    /// The work packet may update the edges.
    ///
    /// Arguments:
    /// * `edges`: A vector of edges.
    fn create_process_edge_roots_work(&mut self, edges: Vec<ES>);

    /// Create work packets to handle nodes pointed by root edges.
    ///
    /// The work packet cannot update root edges, therefore it cannot move the objects.  This
    /// method can only be used by GC algorithms that never moves objects, or GC algorithms that
    /// supports object pinning.
    ///
    /// This method is useful for conservative stack scanning, or VMs that cannot update some
    /// of the root edges.
    ///
    /// Arguments:
    /// * `nodes`: A vector of references to objects pointed by root edges.
    fn create_process_node_roots_work(&mut self, nodes: Vec<ObjectReference>, is_immovable: bool);
}

/// VM-specific methods for scanning roots/objects.
pub trait Scanning<VM: VMBinding> {
    /// Scan stack roots after all mutators are paused.
    const SCAN_MUTATORS_IN_SAFEPOINT: bool = true;

    /// Scan all the mutators within a single work packet.
    ///
    /// `SCAN_MUTATORS_IN_SAFEPOINT` should also be enabled
    const SINGLE_THREAD_MUTATOR_SCANNING: bool = true;

    /// Return true if the given object supports edge enqueuing.
    ///
    /// -   If this returns true, MMTk core will call `scan_object` on the object.
    /// -   Otherwise, MMTk core will call `scan_object_and_trace_edges` on the object.
    ///
    /// For maximum performance, the VM should support edge-enqueuing for as many objects as
    /// practical.  Also note that this method is called for every object to be scanned, so it
    /// must be fast.  The VM binding should avoid expensive checks and keep it as efficient as
    /// possible.  Add `#[inline(always)]` to ensure it is inlined.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    #[inline(always)]
    fn support_edge_enqueuing(_tls: VMWorkerThread, _object: ObjectReference) -> bool {
        true
    }

    /// Delegated scanning of a object, visiting each reference field encountered.
    ///
    /// The VM shall call `edge_visitor.visit_edge` on each reference field.
    ///
    /// The VM may skip a reference field if it holds a null reference.  If the VM supports tagged
    /// references, it must skip tagged reference fields which are not holding references.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `edge_visitor`: Called back for each edge.
    fn scan_object<EV: EdgeVisitor<VM::VMEdge>>(
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut EV,
    );

    /// Delegated scanning of a object, visiting each reference field encountered, and trace the
    /// objects pointed by each field.
    ///
    /// The VM shall call `object_tracer.trace_object` on the value held in each reference field,
    /// and assign the returned value back to the field.  If the VM uses tagged references, the
    /// value passed to `object_tracer.trace_object` shall be the `ObjectReference` to the object
    /// without any tag bits.
    ///
    /// The VM may skip a reference field if it holds a null reference.  If the VM supports tagged
    /// references, it must skip tagged reference fields which are not holding references.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `object_tracer`: Called back for the content of each edge.
    fn scan_object_and_trace_edges<OT: ObjectTracer>(
        _tls: VMWorkerThread,
        _object: ObjectReference,
        _object_tracer: &mut OT,
    ) {
        unreachable!("scan_object_and_trace_edges() will not be called when support_edge_enqueue() is always true.")
    }

    /// MMTk calls this method at the first time during a collection that thread's stacks
    /// have been scanned. This can be used (for example) to clean up
    /// obsolete compiled methods that are no longer being executed.
    ///
    /// Arguments:
    /// * `partial_scan`: Whether the scan was partial or full-heap.
    /// * `tls`: The GC thread that is performing the thread scan.
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: VMWorkerThread);

    /// Scan all the mutators for roots.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_thread_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory<VM::VMEdge>);

    /// Scan one mutator for roots.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `mutator`: The reference to the mutator whose roots will be scanned.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_thread_root(
        tls: VMWorkerThread,
        mutator: &'static mut Mutator<VM>,
        factory: impl RootsWorkFactory<VM::VMEdge>,
    );

    /// Scan VM-specific roots. The creation of all root scan tasks (except thread scanning)
    /// goes here.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_vm_specific_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory<VM::VMEdge>);

    /// Return whether the VM supports return barriers. This is unused at the moment.
    fn supports_return_barrier() -> bool;

    fn prepare_for_roots_re_scanning();
}
