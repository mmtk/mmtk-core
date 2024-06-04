extern crate libc;
extern crate mmtk;

use std::sync::OnceLock;

use mmtk::vm::VMBinding;
use mmtk::MMTK;

pub mod active_plan;
pub mod api;
pub mod collection;
pub mod object_model;
pub mod reference_glue;
pub mod scanning;

mod slots;

#[derive(Default)]
pub struct DummyVM;

// Documentation: https://docs.mmtk.io/api/mmtk/vm/trait.VMBinding.html
impl VMBinding for DummyVM {
    type VMObjectModel = object_model::VMObjectModel;
    type VMScanning = scanning::VMScanning;
    type VMCollection = collection::VMCollection;
    type VMActivePlan = active_plan::VMActivePlan;
    type VMReferenceGlue = reference_glue::VMReferenceGlue;
    type VMSlot = slots::DummyVMSlot;
    type VMMemorySlice = slots::DummyVMMemorySlice;

    /// Allowed maximum alignment in bytes.
    const MAX_ALIGNMENT: usize = 1 << 6;
}

pub static SINGLETON: OnceLock<Box<MMTK<DummyVM>>> = OnceLock::new();

fn mmtk() -> &'static MMTK<DummyVM> {
    SINGLETON.get().unwrap()
}
