// TODO: We should fix missing docs for public items and turn this on (Issue #309).
// #![deny(missing_docs)]

// Allow this for now. Clippy suggests we should use Sft, Mmtk, rather than SFT and MMTK.
// According to its documentation (https://rust-lang.github.io/rust-clippy/master/index.html#upper_case_acronyms),
// with upper-case-acronyms-aggressive turned on, it should also warn us about SFTMap, VMBinding, GCWorker.
// However, it seems clippy does not catch all these patterns at the moment. So it would be hard for us to
// find all the patterns and consistently change all of them. I think it would be a better idea to just allow this.
// We may reconsider this in the future. Plus, using upper case letters for acronyms does not sound a big issue
// to me - considering it will break our API and all the efforts for all the developers to make the change, it may
// not worth it.
#![allow(clippy::upper_case_acronyms)]

//! Memory Management ToolKit (MMTk) is a portable and high performance memory manager
//! that includes various garbage collection algorithms and provides clean and efficient
//! interfaces to cooperate with language implementations. MMTk features highly modular
//! and highly reusable designs. It includes components such as allocators, spaces and
//! work packets that GC implementers can choose from to compose their own GC plan easily.
//!
//! Logically, this crate includes these major parts:
//! * GC components:
//!   * [Allocators](util/alloc/allocator/trait.Allocator.html): handlers of allocation requests which allocate objects to the bound space.
//!   * [Policies](policy/space/trait.Space.html): definitions of semantics and behaviors for memory regions.
//!      Each space is an instance of a policy, and takes up a unique proportion of the heap.
//!   * [Work packets](scheduler/work/trait.GCWork.html): units of GC work scheduled by the MMTk's scheduler.
//! * [GC plans](plan/global/trait.Plan.html): GC algorithms composed from components.
//! * [Heap implementations](util/heap/index.html): the underlying implementations of memory resources that support spaces.
//! * [Scheduler](scheduler/scheduler/struct.GCWorkScheduler.html): the MMTk scheduler to allow flexible and parallel execution of GC work.
//! * Interfaces: bi-directional interfaces between MMTk and language implementations
//!   i.e. [the memory manager API](memory_manager/index.html) that allows a language's memory manager to use MMTk
//!   and [the VMBinding trait](vm/trait.VMBinding.html) that allows MMTk to call the language implementation.

extern crate libc;
extern crate strum_macros;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[cfg(target = "x86_64-unknown-linux-gnu")]
extern crate atomic;
extern crate atomic_traits;
extern crate crossbeam;
extern crate num_cpus;
#[macro_use]
extern crate downcast_rs;
#[macro_use]
extern crate static_assertions;
#[macro_use]
extern crate probe;

mod mmtk;
pub use mmtk::MMTKBuilder;
pub(crate) use mmtk::MMAPPER;
pub use mmtk::MMTK;

mod global_state;

mod policy;

pub mod build_info;
pub mod memory_manager;
pub mod plan;
pub mod scheduler;
pub mod util;
pub mod vm;

pub use crate::plan::{
    AllocationSemantics, BarrierSelector, Mutator, MutatorContext, ObjectQueue, Plan,
};
pub use crate::policy::copy_context::PolicyCopyContext;
