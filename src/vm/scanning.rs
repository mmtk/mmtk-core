use crate::plan::Mutator;
use crate::scheduler::GCWorker;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::slot::Slot;
use crate::vm::VMBinding;

/// Callback trait of scanning functions that report slots.
pub trait SlotVisitor<SL: Slot> {
    /// Call this function for each slot.
    fn visit_slot(&mut self, slot: SL);
}

/// This lets us use closures as SlotVisitor.
impl<SL: Slot, F: FnMut(SL)> SlotVisitor<SL> for F {
    fn visit_slot(&mut self, slot: SL) {
        #[cfg(debug_assertions)]
        trace!(
            "(FunctionClosure) Visit slot {:?} (pointing to {:?})",
            slot,
            slot.load()
        );
        self(slot)
    }
}

/// Callback trait of scanning functions that directly trace through object graph edges.
pub trait ObjectTracer {
    /// Call this function to trace through an object graph edge which points to `object`.
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

/// This type specifies how object-scanning functions ([`Scanning::scan_object`] and
/// [`Scanning::scan_object_and_trace_edges`]) should handle strong and weak reference fields.
///
/// Note that it is the VM and the VM binding that ultimately decides *which* reference is strong
/// and *which* reference is weak.  Particularly, the VM binding is allowed to conservatively report
/// weak references as strong.  For example,
///
/// -   A VM binding can report all weak references as strong during nursery collections or
///     concurrent collections to avoid expensive weak reference processing.
/// -   The VM binding of a JVM (e.g. mmtk-openjdk) can report the weak reference field in
///     `SoftReference` as strong during non-emergency GCs, and weak during emergency GCs.
pub trait RefScanPolicy {
    /// True if the reference scanning function should visit strong reference fields in the object using
    /// callbacks.
    const SHOULD_VISIT_STRONG: bool;
    /// True if the reference scanning function should visit weak reference fields in the object using
    /// callbacks.
    const SHOULD_VISIT_WEAK: bool;
    /// True if the reference scanning function should discover weak reference fields in VM-specific
    /// ways.
    const SHOULD_DISCOVER_WEAK: bool;
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
pub trait RootsWorkFactory<SL: Slot>: Clone + Send + 'static {
    // TODO:
    // 1.  Rename the functions and remove the repeating `create_process_` and `_work`.
    // 2.  Rename the functions to reflect both the form (slots / nodes) and the semantics (pinning
    //     / transitive pinning / non-pinning) of each function.
    // 3.  Introduce a function to give the VM binding a way to update root edges without
    //     representing the roots as slots.  See: https://github.com/mmtk/mmtk-core/issues/710

    /// Create work packets to handle non-pinned roots.  The roots are represented as slots so that
    /// they can be updated.
    ///
    /// The work packet may update the slots.
    ///
    /// Arguments:
    /// * `slots`: A vector of slots.
    fn create_process_roots_work(&mut self, slots: Vec<SL>);

    /// Create work packets to handle non-transitively pinning roots.
    ///
    /// The work packet will prevent the objects in `nodes` from moving,
    /// i.e. they will be pinned for the duration of the GC.
    /// But it will not prevent the children of those objects from moving.
    ///
    /// This method is useful for conservative stack scanning, or VMs that cannot update some
    /// of the root slots.
    ///
    /// Arguments:
    /// * `nodes`: A vector of references to objects pointed by edges from roots.
    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>);

    /// Create work packets to handle transitively pinning (TP) roots.
    ///
    /// Similar to `create_process_pinning_roots_work`, this work packet will not move objects in `nodes`.
    /// Unlike `create_process_pinning_roots_work`, no objects in the transitive closure of `nodes` will be moved, either.
    ///
    /// Arguments:
    /// * `nodes`: A vector of references to objects pointed by edges from roots.
    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>);
}

/// VM-specific methods for scanning roots/objects.
pub trait Scanning<VM: VMBinding> {
    /// When set to `true`, all plans will guarantee that during each GC, each live object is
    /// enqueued at most once, and therefore scanned (by either [`Scanning::scan_object`] or
    /// [`Scanning::scan_object_and_trace_edges`]) at most once.
    ///
    /// When set to `false`, MMTk may enqueue an object multiple times due to optimizations, such as
    /// using non-atomic operatios to mark objects.  Consequently, an object may be scanned multiple
    /// times during a GC.
    ///
    /// The default value is `false` because duplicated object-enqueuing is benign for most VMs, and
    /// related optimizations, such as non-atomic marking, can improve GC speed. VM bindings can
    /// override this if they need.  For example, some VMs piggyback on object-scanning to visit
    /// objects during a GC, but may have data race if multiple GC workers visit the same object at
    /// the same time.  Such VMs can set this constant to `true` to workaround this problem.
    const UNIQUE_OBJECT_ENQUEUING: bool = false;

    /// Return true if the given object supports slot enqueuing.
    ///
    /// -   If this returns true, MMTk core will call `scan_object` on the object.
    /// -   Otherwise, MMTk core will call `scan_object_and_trace_edges` on the object.
    ///
    /// For maximum performance, the VM should support slot-enqueuing for as many objects as
    /// practical.  Also note that this method is called for every object to be scanned, so it
    /// must be fast.  The VM binding should avoid expensive checks and keep it as efficient as
    /// possible.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    fn support_slot_enqueuing(_tls: VMWorkerThread, _object: ObjectReference) -> bool {
        true
    }

    /// Delegated scanning of a object, visiting each reference field encountered.
    ///
    /// The VM shall call `slot_visitor.visit_slot` on each reference field.  This effectively
    /// visits all outgoing edges from the current object in the form of slots.
    ///
    /// The VM may skip a reference field if it is not holding an object reference (e.g. if the
    /// field is holding a null reference, or a tagged non-reference value such as small integer).
    /// Even if not skipped, [`Slot::load`] will still return `None` if the slot is not holding an
    /// object reference.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `slot_visitor`: Called back for each field.
    fn scan_object<SV: SlotVisitor<VM::VMSlot>, R: RefScanPolicy>(
        tls: VMWorkerThread,
        object: ObjectReference,
        slot_visitor: &mut SV,
    );

    /// Delegated scanning of a object, visiting each reference field encountered, and tracing the
    /// objects pointed by each field.
    ///
    /// The VM shall call `object_tracer.trace_object` with the argument being the object reference
    /// held in each reference field.  If the GC moves the object, the VM shall update the field so
    /// that it refers to the object using the object reference returned from `trace_object`.  This
    /// effectively traces through all outgoing edges from the current object directly.
    ///
    /// The VM must skip reference fields that are not holding object references (e.g. if the
    /// field is holding a null reference, or a tagged non-reference value such as small integer).
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `tls`: The VM-specific thread-local storage for the current worker.
    /// * `object`: The object to be scanned.
    /// * `object_tracer`: Called back for the object reference held in each field.
    fn scan_object_and_trace_edges<OT: ObjectTracer, R: RefScanPolicy>(
        _tls: VMWorkerThread,
        _object: ObjectReference,
        _object_tracer: &mut OT,
    ) {
        unreachable!("scan_object_and_trace_edges() will not be called when support_slot_enqueuing() is always true.")
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
        factory: impl RootsWorkFactory<VM::VMSlot>,
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
    fn scan_vm_specific_roots(tls: VMWorkerThread, factory: impl RootsWorkFactory<VM::VMSlot>);

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
    /// This function is called in a GC after the transitive closure from roots is computed, that
    /// is, all reachable objects from roots are reached.  This function gives the VM binding an
    /// opportunitiy to process finalizers and weak references.
    ///
    /// MMTk core enables the VM binding to do the following in this function:
    ///
    /// 1.  Query if an object is already reached.
    ///     -   by calling `ObjectReference::is_reachable()`
    /// 2.  Get the new address of an object if it is already reached.
    ///     -   by calling `ObjectReference::get_forwarded_object()`
    /// 3.  Keep an object and its descendents alive if not yet reached.
    ///     -   using `tracer_context`
    /// 4.  Request this function to be called again after transitive closure is finished again.
    ///     -   by returning `true`
    ///
    /// The `tracer_context` parameter provides the VM binding the mechanism for retaining
    /// unreachable objects (i.e. keeping them alive in this GC).  The following snippet shows a
    /// typical use case of handling finalizable objects for a Java-like language.
    ///
    /// ```rust
    /// let finalizable_objects: Vec<ObjectReference> = my_vm::get_finalizable_object();
    /// let mut new_finalizable_objects = vec![];
    ///
    /// tracer_context.with_tracer(worker, |tracer| {
    ///     for object in finalizable_objects {
    ///         if object.is_reachable() {
    ///             // `object` is still reachable.
    ///             // It may have been moved if it is a copying GC.
    ///             let new_object = object.get_forwarded_object().unwrap_or(object);
    ///             new_finalizable_objects.push(new_object);
    ///         } else {
    ///             // `object` is unreachable.
    ///             // Retain it, and enqueue it for postponed finalization.
    ///             let new_object = tracer.trace_object(object);
    ///             my_vm::enqueue_finalizable_object_to_be_executed_later(new_object);
    ///         }
    ///     }
    /// });
    /// ```
    ///
    /// Within the closure `|tracer| { ... }`, the VM binding can call `tracer.trace_object(object)`
    /// to retain `object` and get its new address if moved.  After `with_tracer` returns, it will
    /// create work packets in the `VMRefClosure` work bucket to compute the transitive closure from
    /// the objects retained in the closure.
    ///
    /// The `memory_manager::is_mmtk_object` function can be used in this function if
    /// -   the "is_mmtk_object" feature is enabled, and
    /// -   `VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING` is true.
    ///
    /// Arguments:
    /// * `worker`: The current GC worker.
    /// * `tracer_context`: Use this to get access an `ObjectTracer` and use it to retain and update
    ///   weak references.
    ///
    /// If `process_weak_refs` returns `true`, then `process_weak_refs` will be called again after
    /// all work packets in the `VMRefClosure` work bucket has been executed, by which time all
    /// objects reachable from the objects retained in this function will have been reached.
    ///
    /// # Performance notes
    ///
    /// **Retain as many objects as needed in one invocation of `tracer_context.with_tracer`, and
    /// avoid calling `with_tracer` again and again** for each object.  The `tracer` provided by
    /// `ObjectTracerFactory::with_tracer` enqueues retained objects in an internal list specific to
    /// this invocation of `with_tracer`, and will create reasonably sized work packets to compute
    /// the transitive closure.  This means the invocation of `with_tracer` has a non-trivial
    /// overhead, but each invocation of `tracer.trace_object` is cheap.
    ///
    /// *Don't do this*:
    ///
    /// ```rust
    /// for object in objects {
    ///     tracer_context.with_tracer(worker, |tracer| { // This is expensive! DON'T DO THIS!
    ///         tracer.trace_object(object);
    ///     });
    /// }
    /// ```
    ///
    /// **Use `ObjectReference::get_forwarded_object()` to get the forwarded address of reachable
    /// objects.  Only use `tracer.trace_object` for retaining unreachable objects.** If
    /// `trace_object` is called on an already reached object, it will also return its new address
    /// if moved. However, `tracer_context.with_tracer` has a cost, and the VM binding may
    /// accidentally "resurrect" dead objects if failed to check `object.is_reachable()` first. If
    /// the VM binding does not intend to retain any objects, it should completely avoid touching
    /// `tracer_context`.
    ///
    /// **Clone the `tracer_context` for parallelism.**  The `ObjectTracerContext` has `Clone` as
    /// its supertrait.  The VM binding can clone it and distribute each clone into a work packet.
    /// By doing so, the VM binding can parallelize the processing of finalizers and weak references
    /// by creating multiple work packets.
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
