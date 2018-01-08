mod object_model;
mod scanning;
pub use self::object_model::ObjectModel;
pub use self::scanning::Scanning;

#[cfg(feature = "jikesrvm")]
mod jikesrvm;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::*;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::object_model::VMObjectModel as VMObjectModel;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::scanning::VMScanning as VMScanning;

#[cfg(not(feature = "jikesrvm"))]
mod openjdk;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::*;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::object_model::VMObjectModel as VMObjectModel;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::scanning::VMScanning as VMScanning;