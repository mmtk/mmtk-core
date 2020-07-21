#![feature(asm)]
#![feature(const_fn)]
#![feature(integer_atomics)]
#![feature(drain_filter)]
#![feature(nll)]
#![feature(box_syntax)]

#[macro_use]
extern crate custom_derive;
#[macro_use]
extern crate enum_derive;

extern crate libc;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate atomic;
extern crate atomic_traits;
extern crate crossbeam_deque;
extern crate num_cpus;

#[macro_use]
pub mod util;
mod mm;
mod mmtk;
mod plan;
pub mod policy;
pub mod vm;

pub use crate::mm::memory_manager;
pub use crate::mmtk::MMTK;
pub use crate::plan::selected_plan::{
    SelectedCollector, SelectedConstraints, SelectedMutator, SelectedPlan, SelectedTraceLocal,
};
pub use crate::plan::{
    Allocator, CollectorContext, MutatorContext, ParallelCollector, Plan, TraceLocal,
    TransitiveClosure,
};
