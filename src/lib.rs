#![allow(static_mut_refs)]
// Use the `{likely, unlikely}` provided by compiler when using nightly
#![cfg_attr(feature = "nightly", feature(core_intrinsics))]

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
//!     Each space is an instance of a policy, and takes up a unique proportion of the heap.
//!   * [Work packets](scheduler/work/trait.GCWork.html): units of GC work scheduled by the MMTk's scheduler.
//! * [GC plans](plan/global/trait.Plan.html): GC algorithms composed from components.
//! * [Heap implementations](util/heap/index.html): the underlying implementations of memory resources that support spaces.
//! * [Scheduler](scheduler/scheduler/struct.GCWorkScheduler.html): the MMTk scheduler to allow flexible and parallel execution of GC work.
//! * Interfaces: bi-directional interfaces between MMTk and language implementations
//!   i.e. [the memory manager API](memory_manager/index.html) that allows a language's memory manager to use MMTk
//!   and [the VMBinding trait](vm/trait.VMBinding.html) that allows MMTk to call the language implementation.

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate downcast_rs;
#[macro_use]
extern crate static_assertions;
#[macro_use]
extern crate probe;

#[macro_use]
pub mod gc_log;
mod mmtk;
pub use mmtk::MMTKBuilder;
use std::{
    cell::UnsafeCell,
    collections::HashMap,
    fs::File,
    io::Write,
    ops::Deref,
    ptr::{addr_of, addr_of_mut},
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        Arc,
    },
    time::{Instant, SystemTime},
};

use atomic::{Atomic, Ordering};
use crossbeam::queue::SegQueue;
pub(crate) use mmtk::MMAPPER;
pub use mmtk::MMTK;
use plan::immix::Pause;
use spin::{Lazy, Mutex};
type RwLock<T> = spin::rwlock::RwLock<T>;

mod global_state;
pub use crate::global_state::LiveBytesStats;

#[macro_use]
mod policy;

pub mod args;
pub mod build_info;
pub mod memory_manager;
pub mod plan;
pub mod scheduler;
pub mod util;
pub mod vm;

pub use crate::plan::{
    AllocationSemantics, BarrierSelector, Mutator, MutatorContext, ObjectQueue, Plan,
};

static NUM_CONCURRENT_TRACING_PACKETS: AtomicUsize = AtomicUsize::new(0);

pub struct LazySweepingJobsCounter {
    decs_counter: Option<Arc<AtomicUsize>>,
    counter: Arc<AtomicUsize>,
}
impl LazySweepingJobsCounter {
    pub fn new() -> Self {
        let lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.read();
        let counter = lazy_sweeping_jobs.curr_counter.as_ref().unwrap();
        counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: None,
            counter: counter.clone(),
        }
    }

    pub fn new_decs() -> Self {
        let lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.read();
        let decs_counter = lazy_sweeping_jobs.curr_decs_counter.as_ref().unwrap();
        decs_counter.fetch_add(1, Ordering::SeqCst);
        let counter = lazy_sweeping_jobs.curr_counter.as_ref().unwrap();
        counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: Some(decs_counter.clone()),
            counter: counter.clone(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> Self {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: None,
            counter: self.counter.clone(),
        }
    }

    pub fn clone_with_decs(&self) -> Self {
        self.decs_counter
            .as_ref()
            .unwrap()
            .fetch_add(1, Ordering::SeqCst);
        self.counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: self.decs_counter.clone(),
            counter: self.counter.clone(),
        }
    }
}

impl Drop for LazySweepingJobsCounter {
    fn drop(&mut self) {
        let lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.read();
        if let Some(decs) = self.decs_counter.as_ref() {
            if decs.fetch_sub(1, Ordering::SeqCst) == 1 {
                let f = lazy_sweeping_jobs.end_of_decs.as_ref().unwrap();
                f(self.clone())
            }
        }
        if self.counter.fetch_sub(1, Ordering::SeqCst) == 1 {
            if let Some(f) = lazy_sweeping_jobs.end_of_lazy.as_ref() {
                f()
            }
        }
    }
}

pub struct LazySweepingJobs {
    prev_decs_counter: Option<Arc<AtomicUsize>>,
    curr_decs_counter: Option<Arc<AtomicUsize>>,
    prev_counter: Option<Arc<AtomicUsize>>,
    curr_counter: Option<Arc<AtomicUsize>>,
    pub end_of_decs: Option<Box<dyn Send + Sync + Fn(LazySweepingJobsCounter)>>,
    pub end_of_lazy: Option<Box<dyn Send + Sync + Fn()>>,
}

impl LazySweepingJobs {
    fn new() -> Self {
        Self {
            prev_decs_counter: None,
            curr_decs_counter: None,
            prev_counter: None,
            curr_counter: None,
            end_of_decs: None,
            end_of_lazy: None,
        }
    }

    pub fn all_finished() -> bool {
        LAZY_SWEEPING_JOBS
            .read()
            .prev_counter
            .as_ref()
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
            == 0
    }

    pub fn swap(&mut self) {
        self.prev_decs_counter = self.curr_decs_counter.take();
        self.curr_decs_counter = Some(Arc::new(AtomicUsize::new(0)));
        self.prev_counter = self.curr_counter.take();
        self.curr_counter = Some(Arc::new(AtomicUsize::new(0)));
    }
}

static LAZY_SWEEPING_JOBS: Lazy<RwLock<LazySweepingJobs>> =
    Lazy::new(|| RwLock::new(LazySweepingJobs::new()));

fn concurrent_marking_packets_drained() -> bool {
    crate::NUM_CONCURRENT_TRACING_PACKETS.load(Ordering::SeqCst) == 0
}

static DISABLE_LASY_DEC_FOR_CURRENT_GC: AtomicBool = AtomicBool::new(false);

fn disable_lasy_dec_for_current_gc() -> bool {
    crate::DISABLE_LASY_DEC_FOR_CURRENT_GC.load(Ordering::SeqCst)
}

#[derive(Debug)]
#[repr(C)]
pub(crate) struct Timer(UnsafeCell<Option<Instant>>);

impl Timer {
    const fn new() -> Self {
        Self(UnsafeCell::new(None))
    }

    pub fn set(&self, instant: Instant) {
        unsafe {
            *self.0.get() = Some(instant);
        }
    }

    pub fn start(&self) {
        unsafe {
            *self.0.get() = Some(Instant::now());
        }
    }
}

unsafe impl Sync for Timer {}

impl Deref for Timer {
    type Target = Instant;

    fn deref(&self) -> &Self::Target {
        let v = unsafe { &*self.0.get() };
        v.as_ref().unwrap()
    }
}

static GC_TRIGGER_TIME: Timer = Timer::new();
static GC_START_TIME: Timer = Timer::new();
static BOOT_TIME: spin::Lazy<SystemTime> = spin::Lazy::new(SystemTime::now);
static GC_EPOCH: AtomicUsize = AtomicUsize::new(0);
static RESERVED_PAGES_AT_GC_START: AtomicUsize = AtomicUsize::new(0);
static RESERVED_PAGES_AT_GC_END: AtomicUsize = AtomicUsize::new(0);
static INSIDE_HARNESS: AtomicBool = AtomicBool::new(false);
static SATB_START: Timer = Timer::new();
static PAUSE_CONCURRENT_MARKING: AtomicBool = AtomicBool::new(false);
static MOVE_CONCURRENT_MARKING_TO_STW: AtomicBool = AtomicBool::new(false);

fn boot_time_secs() -> f64 {
    crate::BOOT_TIME.elapsed().unwrap().as_millis() as f64 / 1000f64
}

fn gc_trigger_time_ms() -> f64 {
    crate::GC_TRIGGER_TIME.elapsed().as_micros() as f64 / 1000f64
}

fn gc_start_time_ms() -> f64 {
    crate::GC_START_TIME.elapsed().as_micros() as f64 / 1000f64
}

#[allow(unused)]
fn inside_harness() -> bool {
    crate::INSIDE_HARNESS.load(Ordering::Relaxed)
}
static NO_EVAC: AtomicBool = AtomicBool::new(false);
static REMSET_RECORDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn args() -> &'static crate::args::RuntimeArgs {
    crate::args::RuntimeArgs::get()
}

static VERBOSE: AtomicUsize = AtomicUsize::new(0);

pub fn verbose(level: usize) -> bool {
    VERBOSE.load(Ordering::Relaxed) >= level
}
