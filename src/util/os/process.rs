use std::io::Result;

pub trait Process {
    fn get_process_memory_maps() -> Result<String>;
    fn get_system_total_memory() -> Result<u64> {
        use sysinfo::MemoryRefreshKind;
        use sysinfo::{RefreshKind, System};

        // TODO: Note that if we want to get system info somewhere else in the future, we should
        // refactor this instance into some global struct. sysinfo recommends sharing one instance of
        // `System` instead of making multiple instances.
        // See https://docs.rs/sysinfo/0.29.0/sysinfo/index.html#usage for more info
        //
        // If we refactor the `System` instance to use it for other purposes, please make sure start-up
        // time is not affected.  It takes a long time to load all components in sysinfo (e.g. by using
        // `System::new_all()`).  Some applications, especially short-running scripts, are sensitive to
        // start-up time.  During start-up, MMTk core only needs the total memory to initialize the
        // `Options`.  If we only load memory-related components on start-up, it should only take <1ms
        // to initialize the `System` instance.
        let sys = System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Ok(sys.total_memory())
    }
}
