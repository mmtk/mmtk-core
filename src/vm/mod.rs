use crate::util::constants::*;

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

pub trait VMBinding
where
    Self: Sized + 'static,
{
    type VMObjectModel: ObjectModel<Self>;
    type VMScanning: Scanning<Self>;
    type VMCollection: Collection<Self>;
    type VMActivePlan: ActivePlan<Self>;
    type VMReferenceGlue: ReferenceGlue<Self>;

    const ALIGNMENT_VALUE: usize = 0xdead_beef;
    const LOG_MIN_ALIGNMENT: usize = LOG_BYTES_IN_INT as usize;
    const MIN_ALIGNMENT: usize = 1 << Self::LOG_MIN_ALIGNMENT;
    #[cfg(target_arch = "x86")]
    const MAX_ALIGNMENT_SHIFT: usize = 1 + LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;
    #[cfg(target_arch = "x86_64")]
    const MAX_ALIGNMENT_SHIFT: usize = LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;

    const MAX_ALIGNMENT: usize = Self::MIN_ALIGNMENT << Self::MAX_ALIGNMENT_SHIFT;

    // This value is used to assert if the cursor is reasonable after last allocation.
    // At the end of an allocation, the allocation cursor should be aligned to this value.
    // Note that MMTk does not attempt to do anything to align the cursor to this value, but
    // it merely asserts with this constant.
    const ALLOC_END_ALIGNMENT: usize = 1;
}
