//! MMTk-to-VM interface: the VMBinding trait.
//!
//! This module provides VM-specific traits that serve as MMTK-to-VM interfaces.
//! Each VM binding needs to provide an implementation for each of the traits.
//! MMTk requires the interfaces to be efficient, as some of the methods are called frequently
//! during collection (e.g. the methods for `ObjectModel`). We rely on cross-crate *link-time-optimization*
//! to remove the overhead of MMTk invoking methods on those traits.
//!
//! It is recommended for a VM binding that uses mmtk-core to do the following to ensure LTO is enabled for performance.
//! 1. Add the following section in the manifest file of a VM binding (`Cargo.toml`). This enables LTO for the release build:
//!    ```
//!    [profile.release]
//!    lto = true
//!    ```
//! 2. Make sure that the crate type for a VM binding supports LTO. To our knowledge, `staticlib` and `cdylib` support LTO, and
//!    `rlib` does *not* support LTO.

mod active_plan;
mod collection;
pub(crate) mod object_model;
mod reference_glue;
mod scanning;
pub mod slot;
pub use self::active_plan::ActivePlan;
pub use self::collection::Collection;
pub use self::collection::GCThreadContext;
pub use self::object_model::specs::*;
pub use self::object_model::ObjectModel;
pub use self::reference_glue::Finalizable;
pub use self::reference_glue::ReferenceGlue;
pub use self::scanning::ObjectTracer;
pub use self::scanning::ObjectTracerContext;
pub use self::scanning::RefScanPolicy;
pub use self::scanning::RootsWorkFactory;
pub use self::scanning::Scanning;
pub use self::scanning::SlotVisitor;

#[cfg(test)]
mod tests;

/// Default min alignment 4 bytes
const DEFAULT_LOG_MIN_ALIGNMENT: usize = 2;
/// Default max alignment 8 bytes
const DEFAULT_LOG_MAX_ALIGNMENT: usize = 3;

/// The `VMBinding` trait associates with each trait, and provides VM-specific constants.
pub trait VMBinding
where
    Self: Sized + 'static + Send + Sync + Default,
{
    /// The binding's implementation of [`crate::vm::ObjectModel`].
    type VMObjectModel: ObjectModel<Self>;
    /// The binding's implementation of [`crate::vm::Scanning`].
    type VMScanning: Scanning<Self>;
    /// The binding's implementation of [`crate::vm::Collection`].
    type VMCollection: Collection<Self>;
    /// The binding's implementation of [`crate::vm::ActivePlan`].
    type VMActivePlan: ActivePlan<Self>;
    /// The binding's implementation of [`crate::vm::ReferenceGlue`].
    type VMReferenceGlue: ReferenceGlue<Self>;

    /// The type of slots in this VM.
    type VMSlot: slot::Slot;
    /// The type of heap memory slice in this VM.
    type VMMemorySlice: slot::MemorySlice<SlotType = Self::VMSlot>;

    /// A value to fill in alignment gaps. This value can be used for debugging. Set this value to 0 to skip filling alignment gaps.
    const ALIGNMENT_VALUE: u8 = 0xab;
    /// Allowed minimal alignment in bytes.
    const MIN_ALIGNMENT: usize = 1 << DEFAULT_LOG_MIN_ALIGNMENT;
    /// Allowed maximum alignment in bytes.
    const MAX_ALIGNMENT: usize = 1 << DEFAULT_LOG_MAX_ALIGNMENT;
    /// Does the binding use a non-zero allocation offset? If this is false, we expect the binding
    /// to always use offset === 0 for allocation, and we are able to do some optimization if we know
    /// offset === 0.
    const USE_ALLOCATION_OFFSET: bool = true;

    /// This value is used to assert if the cursor is reasonable after allocations.
    /// At the end of an allocation, the allocation cursor should be aligned to this value.
    /// Note that MMTk does not attempt to do anything to align the cursor to this value, but
    /// it merely asserts with this constant.
    const ALLOC_END_ALIGNMENT: usize = 1;
}
