use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::MMTK;
use crate::{MMAPPER, VM_MAP};
use std::ptr::null_mut;

pub mod active_plan;
pub mod api;
pub mod collection;
pub mod object_model;
pub mod reference_glue;
pub mod scanning;

pub struct DummyVM;

impl VMBinding for DummyVM {
    type VMObjectModel = object_model::VMObjectModel;
    type VMScanning = scanning::VMScanning;
    type VMCollection = collection::VMCollection;
    type VMActivePlan = active_plan::VMActivePlan;
    type VMReferenceGlue = reference_glue::VMReferenceGlue;
}

//#[cfg(feature = "dummyvm")]
lazy_static! {
    pub static ref SINGLETON: MMTK<DummyVM> = MMTK::new(&VM_MAP, &MMAPPER);
}
