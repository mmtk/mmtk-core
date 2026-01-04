use std::io::Result;

pub trait Process {
    /// Return error if unable to get process memory maps.
    /// If unimplemented, just return Ok with an string to indiate that.
    fn get_process_memory_maps() -> Result<String>;
}
