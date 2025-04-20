// Allow this for now. Clippy suggests we should use Sft, Mmtk, rather than SFT and MMTK.
// According to its documentation (https://rust-lang.github.io/rust-clippy/master/index.html#upper_case_acronyms),
// with upper-case-acronyms-aggressive turned on, it should also warn us about SFTMap, VMBinding, GCWorker.
// However, it seems clippy does not catch all these patterns at the moment. So it would be hard for us to
// find all the patterns and consistently change all of them. I think it would be a better idea to just allow this.
// We may reconsider this in the future. Plus, using upper case letters for acronyms does not sound a big issue
// to me - considering it will break our API and all the efforts for all the developers to make the change, it may
// not worth it.
#![allow(clippy::upper_case_acronyms)]
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
//!      Each space is an instance of a policy, and takes up a unique proportion of the heap.
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
// #[macro_use]
// extern crate static_assertions;
#[cfg(feature = "tracing")]
#[macro_use]
extern crate probe;

#[macro_use]
pub mod gc_log;
mod mmtk;
mod rust_mem_counter;
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

struct Counters {
    pub rc: AtomicUsize,
    pub initial_mark: AtomicUsize,
    pub final_mark: AtomicUsize,
    pub cm_early_quit: AtomicUsize,
    pub full: AtomicUsize,
    pub emergency: AtomicUsize,
    pub yield_nanos: Atomic<u128>,
    pub roots_nanos: Atomic<u128>,
    pub satb_nanos: Atomic<u128>,
    pub total_used_pages: AtomicUsize,
    pub min_used_pages: AtomicUsize,
    pub max_used_pages: AtomicUsize,
    pub gc_with_unfinished_lazy_jobs: AtomicUsize,
    pub incs_triggerd: AtomicUsize,
    pub alloc_triggerd: AtomicUsize,
    pub survival_triggerd: AtomicUsize,
    pub overflow_triggerd: AtomicUsize,
    pub rc_during_satb: AtomicUsize,
}

macro_rules! counter_print_keys_and_values {
    ([$this: ident] $($k: literal: $v: expr,)*) => {
        pub fn print_keys(&$this) { $(print!("{}\t", $k);)* }
        pub fn print_values(&$this) { $(print!("{}\t", $v);)* }
    };
}

impl Counters {
    counter_print_keys_and_values! { [self]
        "gc.rc": self.rc.load(Ordering::SeqCst),
        "gc.initial_satb": self.initial_mark.load(Ordering::SeqCst),
        "gc.final_satb": self.final_mark.load(Ordering::SeqCst),
        "gc.full": self.full.load(Ordering::SeqCst),
        "gc.emergency": self.emergency.load(Ordering::SeqCst),
        "cm_early_quit": self.cm_early_quit.load(Ordering::SeqCst),
        "gc_with_unfinished_lazy_jobs": self.gc_with_unfinished_lazy_jobs.load(Ordering::SeqCst),
        "time.yield": self.yield_nanos.load(Ordering::SeqCst) as f64 / 1000000.0,
        "time.roots": self.roots_nanos.load(Ordering::SeqCst) as f64 / 1000000.0,
        "time.satb": self.satb_nanos.load(Ordering::SeqCst) as f64 / 1000000.0,
        "total_used_pages": self.total_used_pages.load(Ordering::SeqCst),
        "min_used_pages": self.min_used_pages.load(Ordering::SeqCst),
        "max_used_pages": self.max_used_pages.load(Ordering::SeqCst),
        "incs_triggerd": self.incs_triggerd.load(Ordering::SeqCst),
        "alloc_triggerd": self.alloc_triggerd.load(Ordering::SeqCst),
        "survival_triggerd": self.survival_triggerd.load(Ordering::SeqCst),
        "overflow_triggerd": self.overflow_triggerd.load(Ordering::SeqCst),
        "rc_during_satb": self.rc_during_satb.load(Ordering::SeqCst),
    }
}

const fn create_counters() -> Counters {
    let mut counters: Counters =
        unsafe { std::mem::transmute([0u8; std::mem::size_of::<Counters>()]) };
    counters.min_used_pages = AtomicUsize::new(usize::MAX);
    counters
}

fn reset_counters() {
    let mut new_counters = create_counters();
    let global = unsafe { &mut *addr_of_mut!(COUNTERS) };
    std::mem::swap(global, &mut new_counters);
}

fn stop_counters() {
    let retired_counters = unsafe { &mut *addr_of_mut!(RETIRED_COUNTERS) };
    let global = unsafe { &mut *addr_of_mut!(COUNTERS) };
    std::mem::swap(global, retired_counters);
}

static mut RETIRED_COUNTERS: Counters = create_counters();
static mut COUNTERS: Counters = create_counters();

fn counters() -> &'static Counters {
    unsafe { &*addr_of!(COUNTERS) }
}

#[derive(Default)]
struct GCStat {
    pub rc_pauses: usize,
    pub alloc_objects: usize,
    pub alloc_volume: usize,
    pub alloc_los_objects: usize,
    pub alloc_los_volume: usize,
    pub promoted_objects: usize,
    pub promoted_volume: usize,
    pub promoted_copy_objects: usize,
    pub promoted_copy_volume: usize,
    pub promoted_los_objects: usize,
    pub promoted_los_volume: usize,
    pub mature_copy_objects: usize,
    pub mature_copy_volume: usize,
    // Dead mature objects
    pub dead_mature_objects: usize,
    pub dead_mature_volume: usize,
    pub dead_mature_los_objects: usize,
    pub dead_mature_los_volume: usize,
    // Dead mature objects (killed by RC)
    pub dead_mature_rc_objects: usize,
    pub dead_mature_rc_volume: usize,
    pub dead_mature_rc_los_objects: usize,
    pub dead_mature_rc_los_volume: usize,
    // Dead mature objects (killed by SATB)
    pub dead_mature_tracing_objects: usize,
    pub dead_mature_tracing_volume: usize,
    pub dead_mature_tracing_los_objects: usize,
    pub dead_mature_tracing_los_volume: usize,
    // Dead mature objects (with stuck RC)
    pub dead_mature_tracing_stuck_objects: usize,
    pub dead_mature_tracing_stuck_volume: usize,
    pub dead_mature_tracing_stuck_los_objects: usize,
    pub dead_mature_tracing_stuck_los_volume: usize,
    // Reclaimed blocks
    pub reclaimed_blocks_nursery: usize,
    pub reclaimed_blocks_mature: usize,
    // Inc counters
    pub inc_objects: usize,
    pub inc_volume: usize,
}

macro_rules! print_keys_and_values {
    ($($n: ident,)*) => {
        #[allow(unused)]
        pub fn print_keys(&self) {
            $(print!("{}\t", stringify!($n));)*
        }
        #[allow(unused)]
        pub fn print_values(&self) {
            $(print!("{}\t", self.$n);)*
        }
        #[allow(unused)]
        pub fn pretty_print(&self) {
            $(println!(" - {} {}", stringify!($n), self.$n);)*
        }
    };
}

impl GCStat {
    print_keys_and_values![
        rc_pauses,
        alloc_objects,
        alloc_volume,
        alloc_los_objects,
        alloc_los_volume,
        promoted_objects,
        promoted_volume,
        promoted_copy_objects,
        promoted_copy_volume,
        promoted_los_objects,
        promoted_los_volume,
        mature_copy_objects,
        mature_copy_volume,
        dead_mature_objects,
        dead_mature_volume,
        dead_mature_los_objects,
        dead_mature_los_volume,
        dead_mature_rc_objects,
        dead_mature_rc_volume,
        dead_mature_rc_los_objects,
        dead_mature_rc_los_volume,
        dead_mature_tracing_objects,
        dead_mature_tracing_volume,
        dead_mature_tracing_los_objects,
        dead_mature_tracing_los_volume,
        dead_mature_tracing_stuck_objects,
        dead_mature_tracing_stuck_volume,
        dead_mature_tracing_stuck_los_objects,
        dead_mature_tracing_stuck_los_volume,
        reclaimed_blocks_nursery,
        reclaimed_blocks_mature,
        inc_objects,
        inc_volume,
    ];
}

#[allow(unused)]
static STAT: Mutex<GCStat> = Mutex::new(GCStat {
    rc_pauses: 0,
    alloc_objects: 0,
    alloc_volume: 0,
    alloc_los_objects: 0,
    alloc_los_volume: 0,
    promoted_objects: 0,
    promoted_volume: 0,
    promoted_copy_objects: 0,
    promoted_copy_volume: 0,
    promoted_los_objects: 0,
    promoted_los_volume: 0,
    mature_copy_objects: 0,
    mature_copy_volume: 0,
    dead_mature_objects: 0,
    dead_mature_volume: 0,
    dead_mature_los_objects: 0,
    dead_mature_los_volume: 0,
    dead_mature_rc_objects: 0,
    dead_mature_rc_volume: 0,
    dead_mature_rc_los_objects: 0,
    dead_mature_rc_los_volume: 0,
    dead_mature_tracing_objects: 0,
    dead_mature_tracing_volume: 0,
    dead_mature_tracing_los_objects: 0,
    dead_mature_tracing_los_volume: 0,
    dead_mature_tracing_stuck_objects: 0,
    dead_mature_tracing_stuck_volume: 0,
    dead_mature_tracing_stuck_los_objects: 0,
    dead_mature_tracing_stuck_los_volume: 0,
    reclaimed_blocks_nursery: 0,
    reclaimed_blocks_mature: 0,
    inc_objects: 0,
    inc_volume: 0,
});

fn stat(f: impl Fn(&mut GCStat)) {
    if !cfg!(feature = "instrumentation") {
        return;
    }
    if !INSIDE_HARNESS.load(Ordering::SeqCst) {
        return;
    }
    f(&mut STAT.lock())
}

fn should_record_pause_time() -> bool {
    cfg!(feature = "pause_time") && INSIDE_HARNESS.load(Ordering::SeqCst)
}

static SRV: SegQueue<(f64, f64)> = SegQueue::new();

fn add_survival_ratio(srv: f64, predict: f64) {
    if cfg!(feature = "survival_ratio") && INSIDE_HARNESS.load(Ordering::SeqCst) {
        SRV.push((srv, predict));
    }
}

fn output_survival_ratios() {
    let headers = ["srv", "predict"];
    let mut s = headers.join(",") + "\n";
    while let Some((a, b)) = SRV.pop() {
        s += &[format!("{:.3}", a), format!("{:.3}", b)].join(",");
        s += "\n";
    }
    let mut file = File::create("scratch/srv.csv").unwrap();
    file.write_all(s.as_bytes()).unwrap();
}

static PAUSE_TIMES: SegQueue<u128> = SegQueue::new();

fn add_pause_time(_pause: Pause, nanos: u128) {
    if should_record_pause_time() {
        PAUSE_TIMES.push(nanos);
    }
}

fn output_pause_time() {
    let mut s = "".to_owned();
    while let Some(record) = PAUSE_TIMES.pop() {
        s += &format!("{}\n", record);
    }
    let mut file = File::create("scratch/pauses.csv").unwrap();
    file.write_all(s.as_bytes()).unwrap();
}

static NO_EVAC: AtomicBool = AtomicBool::new(false);
static REMSET_RECORDING: AtomicBool = AtomicBool::new(false);

pub fn gc_worker_id() -> Option<usize> {
    crate::scheduler::current_worker_ordinal()
}

pub(crate) fn args() -> &'static crate::args::RuntimeArgs {
    crate::args::RuntimeArgs::get()
}

lazy_static! {
    static ref OBJ_COUNT: std::sync::Mutex<HashMap<usize, (usize, usize)>> =
        std::sync::Mutex::new(HashMap::new());
}

fn record_obj(size: usize) {
    assert!(cfg!(feature = "object_size_distribution"));
    let mut counts = OBJ_COUNT.lock().unwrap();
    counts
        .entry(size.next_power_of_two())
        .and_modify(|x| {
            x.0 += 1;
            x.1 += size;
        })
        .or_insert((1, size));
}

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

fn record_live_bytes(size: usize) {
    assert!(cfg!(feature = "lxr_satb_live_bytes_counter"));
    LIVE_BYTES.fetch_add(size, Ordering::SeqCst);
}

fn report_and_reset_live_bytes() {
    assert!(cfg!(feature = "lxr_satb_live_bytes_counter"));
    gc_log!(
        " - live size: {} bytes ({}M)",
        LIVE_BYTES.load(Ordering::SeqCst),
        LIVE_BYTES.load(Ordering::SeqCst) >> 20
    );
    LIVE_BYTES.store(0, Ordering::SeqCst);
}

pub fn dump_and_reset_obj_dist(kind: &str, counts: &mut HashMap<usize, (usize, usize)>) {
    assert!(cfg!(feature = "object_size_distribution"));
    // let mut total_size: usize = 0;
    let mut total_count: usize = 0;
    let mut table = vec![];
    for (size, v) in &*counts {
        // total_size += v.1;
        total_count += v.0;
        table.push((size, v));
    }
    table.sort_by_key(|x| x.0);
    eprintln!("{} Size Distribution:", kind);
    let mut accumulative_count = 0;
    for (size, (count, total)) in table {
        // let curr = size * count;
        accumulative_count += count;
        eprintln!(
            " - obj-size={} ({}) count={} total-size={} accumulative-count={} ({}%)",
            size,
            if *size < (1 << 10) {
                format!("{}B", *size)
            } else if *size < (1 << 20) {
                format!("{}K", *size >> 10)
            } else if *size < (1 << 30) {
                format!("{}M", *size >> 20)
            } else {
                format!("{}G", *size >> 30)
            },
            count,
            total,
            accumulative_count,
            (100 * accumulative_count) as f64 / total_count as f64
        );
    }
    counts.clear();
}

static VERBOSE: AtomicUsize = AtomicUsize::new(0);

pub fn verbose(level: usize) -> bool {
    VERBOSE.load(Ordering::Relaxed) >= level
}

static SANITY_LIVE_SIZE_IX: AtomicUsize = AtomicUsize::new(0);
static SANITY_LIVE_SIZE_LOS: AtomicUsize = AtomicUsize::new(0);
static FRAG_EXP_ENABLED: AtomicBool = AtomicBool::new(false);
fn frag_exp_enabled() -> bool {
    if !cfg!(feature = "periodic_fragmentation_analysis") {
        return true;
    }
    FRAG_EXP_ENABLED.load(Ordering::Relaxed)
}

#[derive(Default)]
#[allow(unused)]
struct LocalRCStat {
    pub total_incs: usize,
    pub los_incs: usize,
    pub ac_incs: usize,
    pub los_ac_incs: usize,
    pub ac_calls: usize,
    pub los_ac_calls: usize,
    pub opw_incs: usize,
    pub opw_calls: usize,
    pub los_opw_incs: usize,
    pub los_opw_calls: usize,
    pub rec_incs: usize,
    pub los_rec_incs: usize,
    pub roots: usize,
}

#[derive(Default)]
#[allow(unused)]
struct RCStat {
    pub total_incs: AtomicUsize,
    pub los_incs: AtomicUsize,
    pub ac_incs: AtomicUsize,
    pub los_ac_incs: AtomicUsize,
    pub ac_calls: AtomicUsize,
    pub los_ac_calls: AtomicUsize,
    pub opw_incs: AtomicUsize,
    pub opw_calls: AtomicUsize,
    pub los_opw_incs: AtomicUsize,
    pub los_opw_calls: AtomicUsize,
    pub rec_incs: AtomicUsize,
    pub los_rec_incs: AtomicUsize,
    pub roots: AtomicUsize,
}

#[allow(unused)]
impl RCStat {
    fn merge(&self, local: &mut LocalRCStat) {
        self.total_incs
            .fetch_add(local.total_incs, Ordering::SeqCst);
        self.los_incs.fetch_add(local.los_incs, Ordering::SeqCst);
        self.ac_incs.fetch_add(local.ac_incs, Ordering::SeqCst);
        self.los_ac_incs
            .fetch_add(local.los_ac_incs, Ordering::SeqCst);
        self.ac_calls.fetch_add(local.ac_calls, Ordering::SeqCst);
        self.los_ac_calls
            .fetch_add(local.los_ac_calls, Ordering::SeqCst);
        self.opw_incs.fetch_add(local.opw_incs, Ordering::SeqCst);
        self.opw_calls.fetch_add(local.opw_calls, Ordering::SeqCst);
        self.los_opw_incs
            .fetch_add(local.los_opw_incs, Ordering::SeqCst);
        self.los_opw_calls
            .fetch_add(local.los_opw_calls, Ordering::SeqCst);
        self.rec_incs.fetch_add(local.rec_incs, Ordering::SeqCst);
        self.los_rec_incs
            .fetch_add(local.los_rec_incs, Ordering::SeqCst);
        self.roots.fetch_add(local.roots, Ordering::SeqCst);
        *local = Default::default();
    }

    fn dump(&self, pause: Pause, pause_time: f64) {
        if pause != Pause::RefCount {
            return;
        }
        eprintln!(
            "<<<RC-STAT>>> {:.3}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}",
            pause_time,
            self.total_incs.load(Ordering::SeqCst),
            self.los_incs.load(Ordering::SeqCst),
            self.ac_incs.load(Ordering::SeqCst),
            self.los_ac_incs.load(Ordering::SeqCst),
            self.ac_calls.load(Ordering::SeqCst),
            self.los_ac_calls.load(Ordering::SeqCst),
            self.opw_incs.load(Ordering::SeqCst),
            self.opw_calls.load(Ordering::SeqCst),
            self.los_opw_incs.load(Ordering::SeqCst),
            self.los_opw_calls.load(Ordering::SeqCst),
            self.rec_incs.load(Ordering::SeqCst),
            self.los_rec_incs.load(Ordering::SeqCst),
            self.roots.load(Ordering::SeqCst),
        );
        self.total_incs.store(0, Ordering::SeqCst);
        self.los_incs.store(0, Ordering::SeqCst);
        self.ac_incs.store(0, Ordering::SeqCst);
        self.los_ac_incs.store(0, Ordering::SeqCst);
        self.ac_calls.store(0, Ordering::SeqCst);
        self.los_ac_calls.store(0, Ordering::SeqCst);
        self.opw_incs.store(0, Ordering::SeqCst);
        self.opw_calls.store(0, Ordering::SeqCst);
        self.los_opw_incs.store(0, Ordering::SeqCst);
        self.los_opw_calls.store(0, Ordering::SeqCst);
        self.rec_incs.store(0, Ordering::SeqCst);
        self.los_rec_incs.store(0, Ordering::SeqCst);
        self.roots.store(0, Ordering::SeqCst);
    }
}

static RC_STAT: RCStat = RCStat {
    total_incs: AtomicUsize::new(0),
    los_incs: AtomicUsize::new(0),
    ac_incs: AtomicUsize::new(0),
    los_ac_incs: AtomicUsize::new(0),
    ac_calls: AtomicUsize::new(0),
    los_ac_calls: AtomicUsize::new(0),
    opw_incs: AtomicUsize::new(0),
    opw_calls: AtomicUsize::new(0),
    los_opw_incs: AtomicUsize::new(0),
    los_opw_calls: AtomicUsize::new(0),
    rec_incs: AtomicUsize::new(0),
    los_rec_incs: AtomicUsize::new(0),
    roots: AtomicUsize::new(0),
};
