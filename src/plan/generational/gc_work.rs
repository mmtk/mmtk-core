use atomic::Ordering;

use crate::plan::generational::global::Gen;
use crate::policy::space::Space;
use crate::scheduler::{gc_work::*, GCWork, GCWorker};
use crate::util::constants::LOG_BYTES_IN_ADDRESS;
use crate::util::metadata::store_metadata;
use crate::util::{Address, ObjectReference};
use crate::vm::edge_shape::Edge;
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

    fn new(edges: Vec<EdgeOf<Self>>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
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
    fn create_scan_work(&self, nodes: Vec<ObjectReference>, roots: bool) -> ScanObjects<Self> {
        ScanObjects::<Self>::new(nodes, false, roots)
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
            store_metadata::<E::VM>(
                &<E::VM as VMBinding>::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
                *obj,
                1,
                None,
                Some(Ordering::SeqCst),
            );
        }
        // scan modbuf only if the current GC is a nursery GC
        if mmtk.plan.is_current_gc_nursery() {
            // Scan objects in the modbuf and forward pointers
            let mut modbuf = vec![];
            ::std::mem::swap(&mut modbuf, &mut self.modbuf);
            GCWork::do_work(
                &mut ScanObjects::<E>::new(modbuf, false, false),
                worker,
                mmtk,
            )
        }
    }
}

/// The array-copy modbuf contains a list of array slices in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet forwards and updates each entry in the recorded slices.
pub struct ProcessArrayCopyModBuf<E: ProcessEdgesWork> {
    modbuf: Vec<(Address, usize)>,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork> ProcessArrayCopyModBuf<E> {
    pub fn new(modbuf: Vec<(Address, usize)>) -> Self {
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessArrayCopyModBuf<E> {
    #[inline(always)]
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Scan modbuf only if the current GC is a nursery GC
        if mmtk.plan.is_current_gc_nursery() {
            // Collect all the entries in all the slices
            let mut edges = vec![];
            for (addr, count) in &self.modbuf {
                for i in 0..*count {
                    edges.push(<E::VM as VMBinding>::VMEdge::from_address(
                        *addr + (i << LOG_BYTES_IN_ADDRESS),
                    ));
                }
            }
            // Forward entries
            GCWork::do_work(&mut E::new(edges, false, mmtk), worker, mmtk)
        }
    }
}
