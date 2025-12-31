use std::io::Result;

pub trait Process {
    fn get_process_memory_maps() -> Result<String>;
    fn get_system_total_memory() -> Result<u64>;
}
