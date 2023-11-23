// This module's code is unused When we compile this module with MMTk core. Allow it.
#![allow(dead_code)]
// Allow missing docs for public items in this module.
#![allow(missing_docs)]

mod gc_work; // Add
mod global;
mod mutator;

pub use self::global::MyGC;
pub use self::global::MYGC_CONSTRAINTS;
