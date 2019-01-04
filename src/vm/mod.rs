mod object_model;
mod scanning;
mod collection;
mod active_plan;
mod reference_glue;
mod memory;
pub mod unboxed_size_constants;
pub use self::object_model::ObjectModel;
pub use self::scanning::Scanning;
pub use self::collection::Collection;
pub use self::active_plan::ActivePlan;
pub use self::reference_glue::ReferenceGlue;
pub use self::memory::Memory;

#[cfg(feature = "jikesrvm")]
pub mod jikesrvm;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::object_model::VMObjectModel as VMObjectModel;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::scanning::VMScanning as VMScanning;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::collection::VMCollection as VMCollection;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::active_plan::VMActivePlan as VMActivePlan;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::reference_glue::VMReferenceGlue as VMReferenceGlue;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::memory::VMMemory as VMMemory;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::JikesRVM;

#[cfg(feature = "openjdk")]
pub mod openjdk;

#[cfg(feature = "openjdk")]
pub use self::openjdk::*;

#[cfg(feature = "openjdk")]
pub use self::openjdk::object_model::VMObjectModel as VMObjectModel;

#[cfg(feature = "openjdk")]
pub use self::openjdk::scanning::VMScanning as VMScanning;

#[cfg(feature = "openjdk")]
pub use self::openjdk::collection::VMCollection as VMCollection;

#[cfg(feature = "openjdk")]
pub use self::openjdk::active_plan::VMActivePlan as VMActivePlan;

#[cfg(feature = "openjdk")]
pub use self::openjdk::reference_glue::VMReferenceGlue as VMReferenceGlue;

#[cfg(feature = "openjdk")]
pub use self::openjdk::memory::VMMemory as VMMemory;
