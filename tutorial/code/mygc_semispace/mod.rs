// This module's code is unused When we compile this module with MMTk core. Allow it.
#![allow(dead_code)]

mod gc_work; // Add
mod global;
mod mutator;

pub use self::global::MyGC;
pub use self::global::MYGC_CONSTRAINTS;
