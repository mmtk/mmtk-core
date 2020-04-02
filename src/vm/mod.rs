mod active_plan;
mod collection;
mod object_model;
mod reference_glue;
mod scanning;
pub mod unboxed_size_constants;
pub use self::active_plan::ActivePlan;
pub use self::collection::Collection;
pub use self::object_model::ObjectModel;
pub use self::reference_glue::ReferenceGlue;
pub use self::scanning::Scanning;

#[cfg(any(test, feature = "dummyvm"))]
pub mod dummyvm;

pub trait VMBinding
where
    Self: Sized + 'static,
{
    type VMObjectModel: ObjectModel<Self>;
    type VMScanning: Scanning<Self>;
    type VMCollection: Collection<Self>;
    type VMActivePlan: ActivePlan<Self>;
    type VMReferenceGlue: ReferenceGlue<Self>;
}
