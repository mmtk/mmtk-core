extern crate libc;
extern crate mmtk;
#[macro_use]
extern crate lazy_static;

use mmtk::vm::VMBinding;
use mmtk::MMTKBuilder;
use mmtk::MMTK;

pub mod api;
pub mod test_fixtures;

mod edges;
#[cfg(test)]
mod tests;

use edges::*;
use mmtk::vm::prelude::*;
use mmtk::vm::GCThreadContext;

// This is intentionally set to a non-zero value to see if it breaks.
// Change this if you want to test other values.
pub const OBJECT_REF_OFFSET: usize = 4;

#[derive(Default)]
pub struct DummyVM;

impl VMBinding for DummyVM {
    type VMEdge = edges::DummyVMEdge;
    type VMMemorySlice = edges::DummyVMMemorySlice;

    /// Allowed maximum alignment in bytes.
    const MAX_ALIGNMENT: usize = 1 << 6;

    fn number_of_mutators() -> usize {
        unimplemented!()
    }

    fn is_mutator(_tls: VMThread) -> bool {
        // FIXME
        true
    }

    fn mutator(_tls: VMMutatorThread) -> &'static mut Mutator<DummyVM> {
        unimplemented!()
    }

    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<DummyVM>> + 'a> {
        unimplemented!()
    }

    fn stop_all_mutators<F>(_tls: VMWorkerThread, _mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<DummyVM>),
    {
        unimplemented!()
    }

    fn resume_mutators(_tls: VMWorkerThread) {
        unimplemented!()
    }

    fn block_for_gc(_tls: VMMutatorThread) {
        panic!("block_for_gc is not implemented")
    }

    fn spawn_gc_thread(_tls: VMThread, _ctx: GCThreadContext<DummyVM>) {}

    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::in_header(0);
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec =
        VMLocalForwardingPointerSpec::in_header(0);
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec =
        VMLocalForwardingBitsSpec::in_header(0);
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::in_header(0);
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec =
        VMLocalLOSMarkNurserySpec::in_header(0);

    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = OBJECT_REF_OFFSET as isize;

    fn copy_object(
        _from: ObjectReference,
        _semantics: CopySemantics,
        _copy_context: &mut GCWorkerCopyContext<DummyVM>,
    ) -> ObjectReference {
        unimplemented!()
    }

    fn copy_object_to(_from: ObjectReference, _to: ObjectReference, _region: Address) -> Address {
        unimplemented!()
    }

    fn get_object_size(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_object_size_when_copied(object: ObjectReference) -> usize {
        Self::get_object_size(object)
    }

    fn get_object_align_when_copied(_object: ObjectReference) -> usize {
        ::std::mem::size_of::<usize>()
    }

    fn get_object_align_offset_when_copied(_object: ObjectReference) -> usize {
        0
    }

    fn get_object_reference_when_copied_to(
        _from: ObjectReference,
        _to: Address,
    ) -> ObjectReference {
        unimplemented!()
    }

    fn ref_to_object_start(object: ObjectReference) -> Address {
        object.to_raw_address().sub(OBJECT_REF_OFFSET)
    }

    fn ref_to_header(object: ObjectReference) -> Address {
        object.to_raw_address()
    }

    fn ref_to_address(object: ObjectReference) -> Address {
        // Just use object start.
        Self::ref_to_object_start(object)
    }

    fn address_to_ref(addr: Address) -> ObjectReference {
        ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET))
    }

    fn dump_object(_object: ObjectReference) {
        unimplemented!()
    }

    type FinalizableType = ObjectReference;

    fn weakref_set_referent(_reference: ObjectReference, _referent: ObjectReference) {
        unimplemented!()
    }
    fn weakref_get_referent(_object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }
    fn weakref_enqueue_references(_references: &[ObjectReference], _tls: VMWorkerThread) {
        unimplemented!()
    }

    fn scan_roots_in_mutator_thread(
        _tls: VMWorkerThread,
        _mutator: &'static mut Mutator<DummyVM>,
        _factory: impl RootsWorkFactory<DummyVMEdge>,
    ) {
        unimplemented!()
    }
    fn scan_vm_specific_roots(_tls: VMWorkerThread, _factory: impl RootsWorkFactory<DummyVMEdge>) {
        unimplemented!()
    }
    fn scan_object<EV: EdgeVisitor<DummyVMEdge>>(
        _tls: VMWorkerThread,
        _object: ObjectReference,
        _edge_visitor: &mut EV,
    ) {
        unimplemented!()
    }
    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: VMWorkerThread) {
        unimplemented!()
    }
    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
    fn prepare_for_roots_re_scanning() {
        unimplemented!()
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// This is used to ensure we initialize MMTk at a specified timing.
pub static MMTK_INITIALIZED: AtomicBool = AtomicBool::new(false);

lazy_static! {
    pub static ref BUILDER: Mutex<MMTKBuilder> = Mutex::new(MMTKBuilder::new());
    pub static ref SINGLETON: MMTK<DummyVM> = {
        let builder = BUILDER.lock().unwrap();
        debug_assert!(!MMTK_INITIALIZED.load(Ordering::SeqCst));
        let ret = mmtk::memory_manager::mmtk_init(&builder);
        MMTK_INITIALIZED.store(true, std::sync::atomic::Ordering::Relaxed);
        *ret
    };
}
