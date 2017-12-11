pub mod scanning;
pub mod scheduler;

#[cfg(feature = "jikesrvm")]
use ::util::address::Address;

#[cfg(feature = "jikesrvm")]
pub static mut JTOC_BASE: Address = Address(0);

#[cfg(feature = "jikesrvm")]
pub mod jtoc;