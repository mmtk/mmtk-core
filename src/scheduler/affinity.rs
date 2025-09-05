use super::worker::ThreadId;
use crate::util::options::AffinityKind;
#[cfg(target_os = "linux")]
use libc::{cpu_set_t, sched_getaffinity, sched_setaffinity, CPU_COUNT, CPU_SET, CPU_ZERO};

/// Represents the ID of a logical CPU on a system.
pub type CoreId = u16;

// XXX: Maybe in the future we can use a library such as https://github.com/Elzair/core_affinity_rs
// to have an OS agnostic way of setting thread affinity.
#[cfg(target_os = "linux")]
/// Return the total number of cores allocated to the program.
pub fn get_total_num_cpus() -> u16 {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        sched_getaffinity(0, std::mem::size_of::<cpu_set_t>(), &mut cs);
        CPU_COUNT(&cs) as u16
    }
}

#[cfg(not(target_os = "linux"))]
/// Return the total number of cores allocated to the program.
pub fn get_total_num_cpus() -> u16 {
    unimplemented!()
}

impl AffinityKind {
    /// Resolve affinity of GC thread. Has a side-effect of calling into the kernel to set the
    /// thread affinity. Note that we assume that each GC thread is equivalent to an OS or hardware
    /// thread.
    pub fn resolve_affinity(&self, thread: ThreadId) {
        match self {
            AffinityKind::OsDefault => {}
            AffinityKind::AllInSet(cpuset) => {
                // Bind the current thread to all the cores in the set
                debug!("Set affinity for thread {} to cpuset {:?}", thread, cpuset);
                bind_current_thread_to_cpuset(cpuset.as_slice());
            }
            AffinityKind::RoundRobin(cpuset) => {
                let cpu = cpuset[thread % cpuset.len()];
                debug!("Set affinity for thread {} to core {}", thread, cpu);
                bind_current_thread_to_core(cpu);
            }
        }
    }
}

#[cfg(target_os = "linux")]
/// Bind the current thread to the specified core.
fn bind_current_thread_to_core(cpu: CoreId) {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        CPU_SET(cpu as usize, &mut cs);
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

#[cfg(not(target_os = "linux"))]
/// Bind the current thread to the specified core.
fn bind_current_thread_to_core(_cpu: CoreId) {
    unimplemented!()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
/// Bind the current thread to the specified core.
fn bind_current_thread_to_cpuset(cpuset: &[CoreId]) {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        for cpu in cpuset {
            CPU_SET(*cpu as usize, &mut cs);
        }
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
/// Bind the current thread to the specified core.
fn bind_current_thread_to_cpuset(_cpuset: &[CoreId]) {
    unimplemented!()
}
