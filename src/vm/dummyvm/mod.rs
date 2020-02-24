use std::ptr::null_mut;

use vm::VMBinding;
use util::OpaquePointer;

pub mod scanning;
pub mod collection;
pub mod object_model;
pub mod active_plan;
pub mod reference_glue;
pub mod api;

pub struct DummyVM;

impl VMBinding for DummyVM {
    type VMObjectModel = object_model::VMObjectModel;
    type VMScanning = scanning::VMScanning;
    type VMCollection = collection::VMCollection;
    type VMActivePlan = active_plan::VMActivePlan;
    type VMReferenceGlue = reference_glue::VMReferenceGlue;
}
