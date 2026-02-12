use crate::util::address::Address;
use crate::util::os::imp::unix_like::unix_common;
use crate::util::os::*;
use std::io::Result;

/// MacOS implementation of the `OS` trait.
pub struct MacOS;

impl OSMemory for MacOS {
    fn dzmmap(
        start: Address,
        size: usize,
        strategy: MmapStrategy,
        _annotation: &MmapAnnotation<'_>,
    ) -> Result<Address> {
        let addr = unix_common::mmap(start, size, strategy)?;

        // Annotation is ignored on macOS
        // Huge page is ignored on macOS

        // Zero memory if we actually reserve the memory
        if strategy.reserve {
            crate::util::memory::zero(start, size);
        }
        Ok(addr)
    }

    fn munmap(start: Address, size: usize) -> Result<()> {
        unix_common::munmap(start, size)
    }

    fn set_memory_access(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
        unix_common::mprotect(start, size, prot)
    }

    fn is_mmap_oom(os_errno: i32) -> bool {
        unix_common::is_mmap_oom(os_errno)
    }

    fn panic_if_unmapped(_start: Address, _size: usize) {
        // Unimplemented for now
    }
}

impl MmapStrategy {
    /// get the flags for POSIX mmap.
    pub fn get_posix_mmap_flags(&self) -> i32 {
        let mut flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED;
        // replace is isgnored on macOS
        if !self.reserve {
            flags |= libc::MAP_NORESERVE;
        }
        flags
    }
}

impl OSProcess for MacOS {
    fn get_process_memory_maps() -> Result<String> {
        // Get the current process ID (replace this with a specific PID if needed)
        let pid = std::process::id();

        // Execute the vmmap command
        let output = std::process::Command::new("vmmap")
            .arg(pid.to_string()) // Pass the PID as an argument
            .output() // Capture the output
            .expect("Failed to execute vmmap command");

        // Check if the command was successful
        if output.status.success() {
            // Convert the command output to a string
            let output_str =
                std::str::from_utf8(&output.stdout).expect("Failed to convert output to string");
            Ok(output_str.to_string())
        } else {
            // Handle the error case
            let error_message = std::str::from_utf8(&output.stderr)
                .expect("Failed to convert error message to string");
            Err(std::io::Error::other(format!(
                "Failed to get process memory map: {}",
                error_message
            )))
        }
    }

    fn get_process_id() -> Result<String> {
        unix_common::get_process_id()
    }

    fn get_thread_id() -> Result<String> {
        unix_common::get_thread_id()
    }

    fn get_total_num_cpus() -> CoreNum {
        unimplemented!()
    }

    fn bind_current_thread_to_core(_core_id: CoreId) {
        unimplemented!()
    }

    fn bind_current_thread_to_cpuset(_core_ids: &[CoreId]) {
        unimplemented!()
    }
}
