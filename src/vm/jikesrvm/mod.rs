mod jtoc;

#[macro_use]
mod jtoc_call;

pub mod scanning;
pub mod scheduler;
pub mod object_model;

use ::util::address::Address;

pub static mut JTOC_BASE: Address = Address(0);