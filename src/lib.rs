#![feature(asm)]
#![feature(const_fn)]
#![feature(integer_atomics)]
#![feature(drain_filter)]
#![feature(nll)]
#![feature(box_syntax)]
#![feature(maybe_uninit_ref)]
#![feature(maybe_uninit_extra)]

//! Memory Management ToolKit (MMTk) is a portable and high performance memory manager
//! that includes various garbage collection algorithms and provides clean and efficient
//! interfaces to cooperate with language implementations. MMTk features highly modular
//! and highly reusable designs. It includes components such as allocators, spaces and
//! work packets that GC implementers can choose from to compose their own GC plan easily.
//!
//! Logically, this crate includes these major parts:
//! * GC components:
//!   * [Allocators](util/alloc/allocator/trait.Allocator.html): handlers of allocation requests which allocate objects to the bound space.
//!   * [Spaces](policy/space/trait.Space.html): memory regions in the heap with defined semantics and behaviors.
//!   * [Work packets](scheduler/work/trait.GCWork.html): units of GC works scheduled by the MMTk's scheduler.
//! * [GC plans](plan/global/trait.Plan.html): GC algorithms composed from components.
//!   *Note that currently the choice of plans is made through Rust features, which is a build-time config, so only one plan is present in the generated binary
//!   and in this documentation. We plan to make this a run-time config so that users can choose GC plans at boot time.*
//! * [Heap implementations](util/heap/index.html): the underlying implementations of memory resources that support spaces.
//! * [Scheduler](scheduler/scheduler/struct.Scheduler.html): the MMTk scheduler to allow flexible and parallel execution of GC works.
//! * Interfaces: bi-directional interfaces between MMTk and language implementations
//!   i.e. [the memory manager API](memory_manager/index.html) that allows a language's memory manager to use MMTk
//!   and [the VMBinding trait](vm/trait.VMBinding.html) that allows MMTk to call the language implementation.

#[macro_use]
extern crate custom_derive;
#[macro_use]
extern crate enum_derive;

extern crate libc;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[cfg(target = "x86_64-unknown-linux-gnu")]
extern crate atomic;
extern crate atomic_traits;
extern crate crossbeam_deque;
extern crate num_cpus;
#[macro_use]
extern crate downcast_rs;

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
    SelectedCollector, SelectedConstraints, SelectedPlan, SelectedTraceLocal,
};
pub use crate::plan::{
    Allocator, CollectorContext, Mutator, MutatorContext, ParallelCollector, Plan, TraceLocal,
    TransitiveClosure,
};
