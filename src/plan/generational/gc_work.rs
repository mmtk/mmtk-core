use atomic::Ordering;

use crate::plan::generational::global::Gen;
use crate::policy::space::Space;
use crate::scheduler::{gc_work::*, GCWork, GCWorker};
use crate::util::ObjectReference;
use crate::vm::edge_shape::{Edge, MemorySlice};
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

/// Process edges for a nursery GC. This type is provided if a generational plan does not use
/// [`crate::scheduler::gc_work::SFTProcessEdges`]. If a plan uses `SFTProcessEdges`,
/// it does not need to use this type.
pub struct GenNurseryProcessEdges<VM: VMBinding> {
    gen: &'static Gen<VM>,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for GenNurseryProcessEdges<VM> {
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;

    fn new(
        edges: Vec<EdgeOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        is_immovable: bool,
    ) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk, is_immovable);
        let gen = base.plan().generational();
        Self { gen, base }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // We cannot borrow `self` twice in a call, so we extract `worker` as a local variable.
        let worker = self.worker();
        self.gen
            .trace_object_nursery(&mut self.base.nodes, object, worker)
    }
    #[inline]
    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        let new_object = self.trace_object(object);
        debug_assert!(!self.gen.nursery.in_space(new_object));
        slot.store(new_object);
    }

    #[inline(always)]
    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        roots: bool,
        is_immovable: bool,
    ) -> ScanObjects<Self> {
        ScanObjects::<Self>::new(nodes, false, roots, is_immovable)
    }
}

impl<VM: VMBinding> Deref for GenNurseryProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for GenNurseryProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// The modbuf contains a list of objects in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet scans the recorded objects and forwards pointers if necessary.
pub struct ProcessModBuf<E: ProcessEdgesWork> {
    modbuf: Vec<ObjectReference>,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork> ProcessModBuf<E> {
    pub fn new(modbuf: Vec<ObjectReference>) -> Self {
        debug_assert!(!modbuf.is_empty());
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessModBuf<E> {
    #[inline(always)]
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Flip the per-object unlogged bits to "unlogged" state.
        for obj in &self.modbuf {
            <E::VM as VMBinding>::VMObjectModel::GLOBAL_LOG_BIT_SPEC.store_atomic::<E::VM, u8>(
                *obj,
                1,
                None,
                Ordering::SeqCst,
            );
        }
        // scan modbuf only if the current GC is a nursery GC
        if mmtk.plan.is_current_gc_nursery() {
            // Scan objects in the modbuf and forward pointers
            let modbuf = std::mem::take(&mut self.modbuf);
            GCWork::do_work(
                &mut ScanObjects::<E>::new(modbuf, false, false, false),
                worker,
                mmtk,
            )
        }
    }
}

/// The array-copy modbuf contains a list of array slices in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet forwards and updates each entry in the recorded slices.
pub struct ProcessRegionModBuf<E: ProcessEdgesWork> {
    /// A list of `(start_address, bytes)` tuple.
    modbuf: Vec<<E::VM as VMBinding>::VMMemorySlice>,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork> ProcessRegionModBuf<E> {
    pub fn new(modbuf: Vec<<E::VM as VMBinding>::VMMemorySlice>) -> Self {
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessRegionModBuf<E> {
    #[inline(always)]
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Scan modbuf only if the current GC is a nursery GC
        if mmtk.plan.is_current_gc_nursery() {
            // Collect all the entries in all the slices
            let mut edges = vec![];
            for slice in &self.modbuf {
                for edge in slice.iter_edges() {
                    edges.push(edge);
                }
            }
            // Forward entries
            GCWork::do_work(&mut E::new(edges, false, mmtk, false), worker, mmtk)
        }
    }
}
