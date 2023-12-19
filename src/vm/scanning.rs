use crate::plan::Mutator;
use crate::scheduler::GCWorker;
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
    /// Call this function to trace through an object graph edge which points to `object`.
    /// `object` must point to a valid object, and cannot be `ObjectReference::NULL`.
    ///
    /// The return value is the new object reference for `object` if it is moved, or `object` if
    /// not moved.  If moved, the caller should update the slot that holds the reference to
    /// `object` so that it points to the new location.
    ///
    /// Note: This function is performance-critical, therefore must be implemented efficiently.
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

/// VM-specific methods for scanning roots/objects.
pub trait Scanning<VM: VMBinding> {
    /// Return true if the given object supports edge enqueuing.
    ///
    /// -   If this returns true, MMTk core will call `scan_object` on the object.
    /// -   Otherwise, MMTk core will call `scan_object_and_trace_edges` on the object.
    ///
    /// For maximum performance, the VM should support edge-enqueuing for as many objects as
    /// practical.  Also note that this method is called for every object to be scanned, so it
    /// must be fast.  The VM binding should avoid expensive checks and keep it as efficient as
    /// possible.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
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
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING` is true.
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
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING` is true.
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

    /// Scan one mutator for stack roots.
    ///
    /// Some VM bindings may not be able to implement this method.
    /// For example, the VM binding may only be able to enumerate all threads and
    /// scan them while enumerating, but cannot scan stacks individually when given
    /// the references of threads.
    /// In that case, it can leave this method empty, and deal with stack
    /// roots in [`Scanning::scan_vm_specific_roots`]. However, in that case, MMTk
    /// does not know those roots are stack roots, and cannot perform any possible
    /// optimization for the stack roots.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `mutator`: The reference to the mutator whose roots will be scanned.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_roots_in_mutator_thread(
        tls: VMWorkerThread,
        mutator: &'static mut Mutator<VM>,
        factory: impl RootsWorkFactory<VM::VMEdge>,
    );

    /// Scan VM-specific roots. The creation of all root scan tasks (except thread scanning)
    /// goes here.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled.
    ///
    /// Arguments:
    /// * `tls`: The GC thread that is performing this scanning.
    /// * `factory`: The VM uses it to create work packets for scanning roots.
    fn scan_vm_specific_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory<VM::VMEdge>);

    /// Return whether the VM supports return barriers. This is unused at the moment.
    fn supports_return_barrier() -> bool;

    /// Prepare for another round of root scanning in the same GC. Some GC algorithms
    /// need multiple transitive closures, and each transitive closure starts from
    /// root scanning. We expect the binding to provide the same root set for every
    /// round of root scanning in the same GC. Bindings can use this call to get
    /// ready for another round of root scanning to make sure that the same root
    /// set will be returned in the upcoming calls of root scanning methods,
    /// such as [`crate::vm::Scanning::scan_roots_in_mutator_thread`] and
    /// [`crate::vm::Scanning::scan_vm_specific_roots`].
    fn prepare_for_roots_re_scanning();

    /// Process weak references.
    ///
    /// This function is called after a transitive closure is completed.
    ///
    /// MMTk core enables the VM binding to do the following in this function:
    ///
    /// 1.  Query if an object is already reached in this transitive closure.
    /// 2.  Get the new address of an object if it is already reached.
    /// 3.  Keep an object and its descendents alive if not yet reached.
    /// 4.  Request this function to be called again after transitive closure is finished again.
    ///
    /// The VM binding can query if an object is currently reached by calling
    /// `ObjectReference::is_reachable()`.
    ///
    /// If an object is already reached, the VM binding can get its new address by calling
    /// `ObjectReference::get_forwarded_object()` as the object may have been moved.
    ///
    /// If an object is not yet reached, the VM binding can keep that object and its descendents
    /// alive.  To do this, the VM binding should use `tracer_context.with_tracer` to get access to
    /// an `ObjectTracer`, and then call its `trace_object(object)` method.  The `trace_object`
    /// method will return the new address of the `object` if it moved the object, or its original
    /// address if not moved.  Implementation-wise, the `ObjectTracer` may contain an internal
    /// queue for newly traced objects, and will flush the queue when `tracer_context.with_tracer`
    /// returns. Therefore, it is recommended to reuse the `ObjectTracer` instance to trace
    /// multiple objects.
    ///
    /// *Note that if `trace_object` is called on an already reached object, the behavior will be
    /// equivalent to `ObjectReference::get_forwarded_object()`.  It will return the new address if
    /// the GC already moved the object when tracing that object, or the original address if the GC
    /// did not move the object when tracing it.  In theory, the VM binding can use `trace_object`
    /// wherever `ObjectReference::get_forwarded_object()` is needed.  However, if a VM never
    /// resurrects objects, it should completely avoid touching `tracer_context`, and exclusively
    /// use `ObjectReference::get_forwarded_object()` to get new addresses of objects.  By doing
    /// so, the VM binding can avoid accidentally resurrecting objects.*
    ///
    /// The VM binding can return `true` from `process_weak_refs` to request `process_weak_refs`
    /// to be called again after the MMTk core finishes transitive closure again from the objects
    /// newly visited by `ObjectTracer::trace_object`.  This is useful if a VM supports multiple
    /// levels of reachabilities (such as Java) or ephemerons.
    ///
    /// Implementation-wise, this function is called as the "sentinel" of the `VMRefClosure` work
    /// bucket, which means it is called when all work packets in that bucket have finished.  The
    /// `tracer_context` expands the transitive closure by adding more work packets in the same
    /// bucket.  This means if `process_weak_refs` returns true, those work packets will have
    /// finished (completing the transitive closure) by the time `process_weak_refs` is called
    /// again.  The VM binding can make use of this by adding custom work packets into the
    /// `VMRefClosure` bucket.  The bucket will be `VMRefForwarding`, instead, when forwarding.
    /// See below.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `worker`: The current GC worker.
    /// * `tracer_context`: Use this to get access an `ObjectTracer` and use it to retain and
    ///   update weak references.
    ///
    /// This function shall return true if this function needs to be called again after the GC
    /// finishes expanding the transitive closure from the objects kept alive.
    fn process_weak_refs(
        _worker: &mut GCWorker<VM>,
        _tracer_context: impl ObjectTracerContext<VM>,
    ) -> bool {
        false
    }

    /// Forward weak references.
    ///
    /// This function will only be called in the forwarding stage when using the mark-compact GC
    /// algorithm.  Mark-compact computes transive closure twice during each GC.  It marks objects
    /// in the first transitive closure, and forward references in the second transitive closure.
    ///
    /// Arguments:
    /// * `worker`: The current GC worker.
    /// * `tracer_context`: Use this to get access an `ObjectTracer` and use it to update weak
    ///   references.
    fn forward_weak_refs(
        _worker: &mut GCWorker<VM>,
        _tracer_context: impl ObjectTracerContext<VM>,
    ) {
    }
}
