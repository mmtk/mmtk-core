use crate::scheduler::GCWorker;
use crate::util::ObjectReference;
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
        #[cfg(debug_assertions)]
        trace!(
            "(FunctionClosure) Visit edge {:?} (pointing to {})",
            edge,
            edge.load()
        );
        self(edge)
    }
}

/// Callback trait of scanning functions that directly trace through edges.
pub trait ObjectTracer {
    /// Call this function for the content of each edge,
    /// and assign the returned value back to the edge.
    ///
    /// Note: This function is performance-critical.
    /// Implementations should consider inlining if necessary.
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference;
}

/// This lets us use closures as ObjectTracer.
impl<F: FnMut(ObjectReference) -> ObjectReference> ObjectTracer for F {
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        self(object)
    }
}

/// An `ObjectTracerContext` gives a GC worker temporary access to an `ObjectTracer`, allowing
/// the GC worker to trace objects.  This trait is intended to abstract out the implementation
/// details of tracing objects, enqueuing objects, and creating work packets that expand the
/// transitive closure, allowing the VM binding to focus on VM-specific parts.
///
/// This trait is used during root scanning and binding-side weak reference processing.
pub trait ObjectTracerContext<VM: VMBinding>: Clone + Send + 'static {
    /// The concrete `ObjectTracer` type.
    ///
    /// FIXME: The current code works because of the unsafe method `ProcessEdgesWork::set_worker`.
    /// The tracer should borrow the worker passed to `with_queuing_tracer` during its lifetime.
    /// For this reason, `TracerType` should have a `<'w>` lifetime parameter.
    /// Generic Associated Types (GAT) is already stablized in Rust 1.65.
    /// We should update our toolchain version, too.
    type TracerType: ObjectTracer;

    /// Create a temporary `ObjectTracer` and provide access in the scope of `func`.
    ///
    /// When the `ObjectTracer::trace_object` is called, if the traced object is first visited
    /// in this transitive closure, it will be enqueued.  After `func` returns, the implememtation
    /// will create work packets to continue computing the transitive closure from the newly
    /// enqueued objects.
    ///
    /// API functions that provide `QueuingTracerFactory` should document
    /// 1.  on which fields the user is supposed to call `ObjectTracer::trace_object`, and
    /// 2.  which work bucket the generated work packet will be added to.  Sometimes the user needs
    ///     to know when the computing of transitive closure finishes.
    ///
    /// Arguments:
    /// -   `worker`: The current GC worker.
    /// -   `func`: A caller-supplied closure in which the created `ObjectTracer` can be used.
    ///
    /// Returns: The return value of `func`.
    fn with_tracer<R, F>(&self, worker: &mut GCWorker<VM>, func: F) -> R
    where
        F: FnOnce(&mut Self::TracerType) -> R;
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

    /// Create work packets to handle non-transitively pinning roots.
    ///
    /// The work packet will prevent the objects in `nodes` from moving,
    /// i.e. they will be pinned for the duration of the GC.
    /// But it will not prevent the children of those objects from moving.
    ///
    /// This method is useful for conservative stack scanning, or VMs that cannot update some
    /// of the root edges.
    ///
    /// Arguments:
    /// * `nodes`: A vector of references to objects pointed by root edges.
    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>);

    /// Create work packets to handle transitively pinning (TP) roots.
    ///
    /// Similar to `create_process_pinning_roots_work`, this work packet will not move objects in `nodes`.
    /// Unlike ``create_process_pinning_roots_work`, no objects in the transitive closure of `nodes` will be moved, either.
    ///
    /// Arguments:
    /// * `nodes`: A vector of references to objects pointed by root edges.
    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>);
}
