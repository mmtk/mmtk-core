pub mod scanning;
pub mod scheduler;

mod jtoc;

use ::util::address::Address;

pub static mut JTOC_BASE: Address = Address(0);