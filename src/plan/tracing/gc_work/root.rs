use std::marker::PhantomData;

use crate::{
    plan::{
        tracing::{
            gc_work::closure::{ProcessNodes, ProcessSlots},
            Trace,
        },
        VectorObjectQueue,
    },
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    util::ObjectReference,
    vm::{RootsKind, RootsWorkFactory, VMBinding},
    MMTK,
};

/// An implementation of [`RootsWorkFactory`] for stop-the-world tracing GC, i.e. finding the
/// transitive closure from roots, with all mutators stopped.
///
/// It creates the [`ProcessSlots`] work packet to handle non-pinning roots, and
/// [`ProcessPinningRoots`] to handle pinning roots (transitive or not).  The work packets will be
/// added to the [`WorkBucketStage::TPinningClosure`], [`WorkBucketStage::PinningRootsTrace`] and
/// [`WorkBucketStage::Closure`] buckets depending on the kinds of roots.
///
/// `DT` and `PT` are the [`Trace`] types for the default trace and pinning trace, respectively.
pub(crate) struct DefaultRootsWorkFactory<VM: VMBinding, DT: Trace<VM = VM>, PT: Trace<VM = VM>> {
    pub(crate) mmtk: &'static MMTK<VM>,
    phantom: PhantomData<(DT, PT)>,
}

impl<VM: VMBinding, DT: Trace<VM = VM>, PT: Trace<VM = VM>> Clone
    for DefaultRootsWorkFactory<VM, DT, PT>
{
    fn clone(&self) -> Self {
        Self {
            mmtk: self.mmtk,
            phantom: PhantomData,
        }
    }
}

impl<VM: VMBinding, DT: Trace<VM = VM>, PT: Trace<VM = VM>> RootsWorkFactory<VM::VMSlot>
    for DefaultRootsWorkFactory<VM, DT, PT>
{
    fn create_process_roots_work(&mut self, slots: Vec<VM::VMSlot>) {
        // Note: We should use the same USDT name "mmtk:roots" for all the three kinds of roots. A
        // VM binding may not call all of the three methods in this impl. For example, the OpenJDK
        // binding only calls `create_process_roots_work`, and the Ruby binding only calls
        // `create_process_pinning_roots_work`. Because `DefaultRootsWorkFactory<VM, DT, PT>` is a
        // generic type, the Rust compiler emits the function bodies on demand, so the resulting
        // machine code may not contain all three USDT trace points.  If they have different names,
        // and our `capture.bt` mentions all of them, `bpftrace` may complain that it cannot find
        // one or more of those USDT trace points in the binary.
        probe!(mmtk, roots, RootsKind::NORMAL, slots.len());

        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_slots(slots.clone());

        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::Closure,
            ProcessSlots::<DT>::new(slots, WorkBucketStage::Closure),
        );
    }

    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::PINNING, nodes.len());

        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_nodes(nodes.clone());

        // Will process roots within the PinningRootsTrace bucket
        // And put work in the Closure bucket
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::PinningRootsTrace,
            ProcessPinningRoots::<VM, PT, DT>::new(nodes, WorkBucketStage::Closure),
        );
    }

    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::TPINNING, nodes.len());

        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_nodes(nodes.clone());

        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::TPinningClosure,
            ProcessPinningRoots::<VM, PT, PT>::new(nodes, WorkBucketStage::TPinningClosure),
        );
    }
}

impl<VM: VMBinding, DT: Trace<VM = VM>, PT: Trace<VM = VM>> DefaultRootsWorkFactory<VM, DT, PT> {
    pub(crate) fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            phantom: PhantomData,
        }
    }
}

/// This work packet processes pinning roots during stop-the-world tracing GC.
///
/// Note that by definition, a "root" is an *edge* from outside the object graph to an object.  This
/// work packet represents each edge as the `ObjectReference` of the object the edge points to.
/// Because pinning roots by definition cannot be updated, we don't need to represent the edges as a
/// [`Slot`].
///
/// [`Slot`]: crate::vm::slot::Slot
///
/// The `roots` member holds a list of `ObjectReference` to objects directly pointed by roots. These
/// objects will be traced using `R2OT` (Root-to-Object Trace).
///
/// After that, it will create work packets for tracing their children.  Those work packets (and the
/// work packets further created by them) will use `O2OT` (Object-to-Object Trace) as their `Trace`
/// implementations.
///
/// Because `roots` are pinning roots, `R2OT` must be a `Trace` that never moves any object.
///
/// The choice of `O2OT` determines whether the `roots` are transitively pinning or not.
///
/// -   If `O2OT` is set to a `Trace` that never moves objects, no descendents of `roots` will be
///     moved in this GC.  That implements transitive pinning roots.
/// -   If `O2OT` may move objects, then this `ProcessRootsNode<VM, R2OT, O2OT>` work packet will
///     only pin the objects in `roots` (because `R2OT` must not move objects anyway), but not their
///     descendents.
pub(crate) struct ProcessPinningRoots<VM: VMBinding, R2OT: Trace<VM = VM>, O2OT: Trace<VM = VM>> {
    phantom: PhantomData<(VM, R2OT, O2OT)>,
    roots: Vec<ObjectReference>,
    bucket: WorkBucketStage,
}

impl<VM: VMBinding, R2OT: Trace<VM = VM>, O2OT: Trace<VM = VM>>
    ProcessPinningRoots<VM, R2OT, O2OT>
{
    pub fn new(nodes: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            phantom: PhantomData,
            roots: nodes,
            bucket,
        }
    }
}

impl<VM: VMBinding, R2OT: Trace<VM = VM>, O2OT: Trace<VM = VM>> GCWork<VM>
    for ProcessPinningRoots<VM, R2OT, O2OT>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        trace!("ProcessPinningRoots");

        let num_roots = self.roots.len();

        // This step conceptually traces the edges from root slots to the objects they point to.
        // However, VMs that deliver root objects instead of root slots are incapable of updating
        // root slots.  Therefore, we call `trace_object` on those objects, and assert the GC
        // doesn't move those objects because we cannot store the updated references back to the
        // slots.
        //
        // The `root_objects_to_scan` variable will hold those root objects which are traced for the
        // first time.  We will create a work packet for scanning those roots.
        let root_objects_to_scan = {
            let mut queue = VectorObjectQueue::new();

            let r2o_trace = R2OT::from_mmtk(mmtk);

            for object in self.roots.iter().copied() {
                let new_object = r2o_trace.trace_object(worker, object, &mut queue);
                debug_assert_eq!(
                    object, new_object,
                    "Object moved while tracing root unmovable root object: {} -> {}",
                    object, new_object
                );
            }

            queue.take()
        };

        let num_enqueued_nodes = root_objects_to_scan.len();
        probe!(mmtk, process_pinning_roots, num_roots, num_enqueued_nodes);

        if !root_objects_to_scan.is_empty() {
            let work = ProcessNodes::<O2OT>::new(root_objects_to_scan, self.bucket);
            worker.add_work(self.bucket, work);
        }

        trace!("ProcessPinningRoots End");
    }
}
