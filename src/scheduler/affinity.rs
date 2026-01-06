use super::worker::ThreadId;
use crate::util::os::*;
use crate::util::options::AffinityKind;

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
                OSProcess::bind_current_thread_to_cpuset(cpuset.as_slice());
            }
            AffinityKind::RoundRobin(cpuset) => {
                let cpu = cpuset[thread % cpuset.len()];
                debug!("Set affinity for thread {} to core {}", thread, cpu);
                OSProcess::bind_current_thread_to_core(cpu);
            }
        }
    }
}
