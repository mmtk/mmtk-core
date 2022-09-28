use super::worker::ThreadId;
use crate::util::options::AffinityKind;
use libc::{cpu_set_t, sched_getaffinity, sched_setaffinity, CPU_COUNT, CPU_SET, CPU_ZERO};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU16, Ordering};

// We use an index variable for the RoundRobin affinity as if we directly use the
// thread id % size(core list) for assiging thread affinities, we can get cases where the GC
// Controller thread gets assigned the same core as another worker. For example, with the core list
// "0,1,2" and 2 GC threads, thread 0 and 1 are assigned cores 0 and 1 respectively, but the GC
// Controller thread (with thread id usize::MAX) is assigned core 0 as well even though there is a
// spare core (core 2).
static CPU_SET_IDX: AtomicU16 = AtomicU16::new(0);

/// Represents the ID of a logical CPU on a system.
pub type CoreId = u16;

// XXX: Maybe in the future we can use a library such as https://github.com/Elzair/core_affinity_rs
// to have an OS agnostic way of setting thread affinity.
#[cfg(target_os = "linux")]
/// Return the total number of allocated cores to the program.
fn get_total_num_cpus() -> u16 {
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        sched_getaffinity(0, std::mem::size_of::<cpu_set_t>(), &mut cs);
        CPU_COUNT(&cs) as u16
    }
}

#[cfg(not(target_os = "linux"))]
/// Return the total number of allocated cores to the program.
fn get_total_num_cpus() -> u16 {
    unimplemented!()
}

// XXX: Maybe using a u128 bitvector with each bit representing a core is more performant?
#[derive(Clone, Debug, Default)]
/// Represents a set of cores. Performs de-duplication of specified cores. Note that the core list
/// is sorted as a side-effect whenever a new core is added to the set.
pub struct CpuSet {
    /// The set of cores
    set: Vec<CoreId>,
    /// Maximum number of cores on the system
    num_cpu: u16,
}

impl CpuSet {
    pub fn new() -> Self {
        CpuSet {
            set: vec![],
            num_cpu: get_total_num_cpus(),
        }
    }

    /// Returns true if the core set is empty.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Add a core to the core list, performing de-duplication. Assumes cores ids on the system are
    /// 0-indexed. Note that this function can panic if the provided core id is greater than the
    /// maximum number of cores on the system. The core list is sorted as a side-effect of this
    /// function.
    pub fn add(&mut self, cpu: CoreId) {
        if cpu >= self.num_cpu {
            panic!(
                "Core id {} greater than maximum number of cores on system",
                cpu
            );
        }

        self.set.push(cpu);
        self.set.sort_unstable();
        self.set.dedup();
    }

    /// Resolve affinity of GC thread. Has a side-effect of calling into the kernel to set the
    /// thread affinity. Note that we assume that each GC thread is equivalent to an OS or hardware
    /// thread.
    pub fn resolve_affinity(&self, thread: ThreadId, affinity: AffinityKind) {
        match affinity {
            AffinityKind::OsDefault => {}
            AffinityKind::Fixed => {
                let cpu = self.set[0];
                info!("Set affinity for thread {} to core {}", thread, cpu);
                bind_current_thread_to_core(cpu);
            }
            AffinityKind::RoundRobin => {
                let idx = CPU_SET_IDX.fetch_add(1, Ordering::SeqCst);
                let cpu = self.set[idx as usize % self.set.len()];
                info!("Set affinity for thread {} to core {}", thread, cpu);
                bind_current_thread_to_core(cpu);
            }
        }
    }
}

#[cfg(target_os = "linux")]
/// Bind the current thread to the specified core.
fn bind_current_thread_to_core(cpu: CoreId) {
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        CPU_SET(cpu as usize, &mut cs);
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

#[cfg(not(target_os = "linux"))]
/// Bind the current thread to the specified core.
fn bind_current_thread_to_core(cpu: CoreId) {
    unimplemented!()
}
