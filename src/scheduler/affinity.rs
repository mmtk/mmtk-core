use super::worker::ThreadId;
use crate::util::options::AffinityKind;

/// Represents the ID of a logical CPU on a system.
pub type CoreId = u16;

/// Return the total number of cores allocated to the program.
pub fn get_total_num_cpus() -> u16 {
    core_affinity::get_core_ids().unwrap().len() as u16
}

impl AffinityKind {
    /// Resolve affinity of GC thread. Has a side-effect of calling into the kernel to set the
    /// thread affinity. Note that we assume that each GC thread is equivalent to an OS or hardware
    /// thread.
    pub fn resolve_affinity(&self, thread: ThreadId) {
        match self {
            AffinityKind::OsDefault => {}
            AffinityKind::RoundRobin(cpuset) => {
                let cpu = cpuset[thread % cpuset.len()];
                debug!("Set affinity for thread {} to core {:?}", thread, cpu);
                bind_current_thread_to_core(cpu);
            }
        }
    }
}

/// Bind the current thread to the specified core.
fn bind_current_thread_to_core(cpu: CoreId) {
    if !core_affinity::set_for_current(core_affinity::CoreId { id: cpu as usize }) {
        panic!("Failed to bind current thread to {:?}", cpu);
    };
}
