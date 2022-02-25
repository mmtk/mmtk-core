#![feature(const_fn_trait_bound)] // for static fixtures

extern crate mmtk;
extern crate libc;
#[macro_use]
extern crate lazy_static;

use mmtk::vm::VMBinding;
use mmtk::MMTK;

pub mod scanning;
pub mod collection;
pub mod object_model;
pub mod active_plan;
pub mod reference_glue;
pub mod api;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct DummyVM;

impl VMBinding for DummyVM {
    type VMObjectModel = object_model::VMObjectModel;
    type VMScanning = scanning::VMScanning;
    type VMCollection = collection::VMCollection;
    type VMActivePlan = active_plan::VMActivePlan;
    type VMReferenceGlue = reference_glue::VMReferenceGlue;

    /// Allowed maximum alignment as shift by min alignment.    
    const MAX_ALIGNMENT_SHIFT: usize = 6_usize - Self::LOG_MIN_ALIGNMENT as usize;

    /// Allowed maximum alignment in bytes.
    const MAX_ALIGNMENT: usize = Self::MIN_ALIGNMENT << Self::MAX_ALIGNMENT_SHIFT;
}

//#[cfg(feature = "dummyvm")]
lazy_static! {
    pub static ref SINGLETON: MMTK<DummyVM> = MMTK::new();
}
