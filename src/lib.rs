#![cfg_attr(feature = "jikesrvm", feature(asm))]

extern crate libc;
#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate log;
extern crate env_logger;

pub mod util;
pub mod vm;
mod policy;
mod plan;
mod mm;

pub use mm::memory_manager::*;