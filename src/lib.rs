#![feature(asm)]
#![feature(const_fn)]
#![feature(integer_atomics)]
#![feature(drain_filter)]
#![feature(nll)]
#![feature(box_syntax)]
#![feature(repr_transparent)]

#[macro_use]
extern crate custom_derive;
#[macro_use]
extern crate enum_derive;

extern crate libc;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate crossbeam_deque;
extern crate num_cpus;
#[macro_use]
extern crate derivative;
extern crate atomic_traits;

#[macro_use]
pub mod util;
pub mod vm;
mod policy;
mod plan;
mod mm;
mod mmtk;
