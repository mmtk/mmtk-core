mod object_model;
mod scanning;
mod collection;
mod active_plan;
mod reference_glue;
pub mod unboxed_size_constants;
pub use self::object_model::ObjectModel;
pub use self::scanning::Scanning;
pub use self::collection::Collection;
pub use self::active_plan::ActivePlan;
pub use self::reference_glue::ReferenceGlue;

pub trait VMBinding
    where
        Self: Sized + 'static
{
    type VMObjectModel: ObjectModel<Self>;
    type VMScanning: Scanning<Self>;
    type VMCollection: Collection<Self>;
    type VMActivePlan: ActivePlan<Self>;
    type VMReferenceGlue: ReferenceGlue<Self>;
}
