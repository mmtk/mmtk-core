#![cfg_attr(feature = "jikesrvm", feature(asm))]
#![feature(const_fn)]
#![feature(const_atomic_usize_new)]
#![feature(const_atomic_bool_new)]
#![feature(const_min_value)]

extern crate libc;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate crossbeam_deque;
extern crate num_cpus;

#[macro_use]
pub mod util;
pub mod vm;
mod policy;
mod plan;
mod mm;

pub use mm::memory_manager::*;
pub use mm::test::*;