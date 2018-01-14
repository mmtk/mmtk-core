mod object_model;
mod scanning;
mod scheduling;
pub use self::object_model::ObjectModel;
pub use self::scanning::Scanning;
pub use self::scheduling::Scheduling;

#[cfg(feature = "jikesrvm")]
pub mod jikesrvm;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::object_model::VMObjectModel as VMObjectModel;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::scanning::VMScanning as VMScanning;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::scheduling::VMScheduling as VMScheduling;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::JikesRVM;

#[cfg(not(feature = "jikesrvm"))]
mod openjdk;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::*;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::object_model::VMObjectModel as VMObjectModel;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::scanning::VMScanning as VMScanning;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::scheduling::VMScheduling as VMScheduling;