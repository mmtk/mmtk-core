mod object_model;

#[cfg(feature = "jikesrvm")]
mod jikesrvm;

#[cfg(feature = "jikesrvm")]
pub use self::jikesrvm::*;

#[cfg(not(feature = "jikesrvm"))]
mod openjdk;

#[cfg(not(feature = "jikesrvm"))]
pub use self::openjdk::*;