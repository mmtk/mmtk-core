pub mod scanning;
pub mod scheduler;
pub mod object_model;

mod jtoc;

use ::util::address::Address;

pub static mut JTOC_BASE: Address = Address(0);