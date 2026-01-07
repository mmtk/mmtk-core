use std::io::Result;

/// Representation of a CPU core identifier.
pub type CoreId = u16;
/// Representation of number of CPU cores.
pub type CoreNum = u16;

/// Abstraction for OS process operations.
pub trait Process {
    /// Get the memory maps for the process. The returned string is a multi-line string.
    /// Fallback: This is only used for debugging. For unimplemented cases, this function can return a placeholder Ok value.
    fn get_process_memory_maps() -> Result<String>;

    /// Get the process ID as a string.
    /// Fallback: This is only used for debugging. For unimplemented cases, this function can return a placeholder Ok value.
    fn get_process_id() -> Result<String>;

    //// Get the thread ID as a string.
    /// Fallback: This is only used for debugging. For unimplemented cases, this function can return a placeholder Ok value.
    fn get_thread_id() -> Result<String>;

    /// Return the total number of cores allocated to the program.
    fn get_total_num_cpus() -> CoreNum;

    /// Bind the current thread to the specified core.
    fn bind_current_thread_to_core(core_id: CoreId);

    /// Bind the current thread to the specified core set.
    fn bind_current_thread_to_cpuset(core_ids: &[CoreId]);
}
