mod jtoc;

#[macro_use]
mod jtoc_call;

pub mod scanning;
pub mod scheduler;
pub mod object_model;
pub mod unboxed_size_constants;

use ::util::address::Address;

pub static mut JTOC_BASE: Address = Address(0);