use std::io::Result;

pub type CoreId = u16;
pub type CoreNum = u16;

pub trait Process {
    /// Return error if unable to get process memory maps.
    /// If unimplemented, just return Ok with an string to indiate that.
    fn get_process_memory_maps() -> Result<String>;

    fn get_process_id() -> Result<String>;

    fn get_thread_id() -> Result<String>;

    fn get_total_num_cpus() -> u16;

    fn bind_current_thread_to_core(core_id: CoreId);

    fn bind_current_thread_to_cpuset(core_ids: &[CoreId]);
}
