use libc::{sched_setaffinity, cpu_set_t, CPU_ZERO, CPU_SET};
use std::mem::MaybeUninit;

/// Identifier of a logical CPU, the numbering is OS-dependant
type CPUID = usize;

/// Assign a number for a thread for mapping the thread to a logical CPU
/// This is **NOT the same** as OS tid
type ThreadID = usize;

/// Different types of affinity for threads
pub enum Affinity {
    /// Let the OS scheduler decide which hardware thread is used for this thread
    OsDefault,
    /// Bind threads to logical CPUs in a round-robin fashion
    RoundRobin,
    OneCPU(CPUID),
}

impl Affinity {
    pub fn resolve_affinity(&self, thread: ThreadID) {
        match self {
            Affinity::OsDefault => {}
            Affinity::RoundRobin => {
                let cpu_id = thread % get_logical_cpu_num() as usize;
                bind_current_thread_to_logical_cpu(cpu_id)
            }
            Affinity::OneCPU(cpu_id) => {
                bind_current_thread_to_logical_cpu(*cpu_id);
            }
        }
    }
}

fn get_logical_cpu_num() -> u16 {
    unsafe {
        let mut si = MaybeUninit::zeroed().assume_init();
        libc::sysinfo(&mut si);
        si.procs
    }
}

fn bind_current_thread_to_logical_cpu(cpu: CPUID) {
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        CPU_SET(cpu, &mut cs);
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(),&cs);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_logical_cpu_num() {
        assert!(get_logical_cpu_num() > 0);
    }
}
