use super::worker::ThreadId;
use crate::util::options::AffinityKind;
use libc::{cpu_set_t, sched_setaffinity, CPU_SET, CPU_ZERO};
use std::mem::MaybeUninit;

pub type CoreId = u16;

#[cfg(target_os = "linux")]
fn get_total_num_cpus() -> u16 {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as u16 }
}

#[cfg(not(target_os = "linux"))]
fn get_total_num_cpus() -> u16 {
    unimplemented!()
}

#[derive(Clone, Debug)]
pub struct CpuSet {
    set: Vec<CoreId>,
    num_cpu: u16,
}

impl CpuSet {
    pub fn new() -> Self {
        CpuSet {
            set: vec![],
            num_cpu: get_total_num_cpus(),
        }
    }

    pub fn add(&mut self, cpu: CoreId) {
        if cpu > self.num_cpu {
            panic!(
                "Core id {} greater than maximum number of cores on system",
                cpu
            );
        }

        self.set.push(cpu);
        self.set.sort();
        self.set.dedup();
    }

    pub fn resolve_affinity(&self, thread: ThreadId, affinity: AffinityKind) {
        match affinity {
            AffinityKind::OsDefault => {}
            AffinityKind::Fixed => {
                if self.set.is_empty() {
                    panic!("Need to provide a list of cores if not using OsDefault affinity");
                }

                let cpu = self.set[0];
                info!("Set affinity for thread {} to core {}", thread, cpu);
                bind_current_thread_to_core(cpu);
            }
            AffinityKind::RoundRobin => {
                if self.set.is_empty() {
                    panic!("Need to provide a list of cores if not using OsDefault affinity");
                }

                let cpu = self.set[thread % self.set.len()];
                info!("Set affinity for thread {} to core {}", thread, cpu);
                bind_current_thread_to_core(cpu);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn bind_current_thread_to_core(cpu: CoreId) {
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        CPU_SET(cpu as usize, &mut cs);
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

#[cfg(not(target_os = "linux"))]
fn bind_current_thread_to_core(cpu: CoreId) {
    unimplemented!()
}
