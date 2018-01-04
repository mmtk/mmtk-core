mod object_model;
pub use self::object_model::ObjectModel;

#[cfg(feature = "jikesrvm")]
mod jikesrvm;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::*;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::object_model::JikesRVMObjectModel as VMObjectModel;

#[cfg(not(feature = "jikesrvm"))]
mod openjdk;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::*;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::object_model::OpenJDKObjectModel as VMObjectModel;