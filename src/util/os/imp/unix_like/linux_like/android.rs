use crate::util::address::Address;
use crate::util::os::imp::unix_like::linux_like::linux_common;
use crate::util::os::imp::unix_like::unix_common;
use crate::util::os::*;

use std::io::Result;

/// Android implementation of the `OS` trait.
pub struct Android;

impl OSMemory for Android {
    fn dzmmap(
        start: Address,
        size: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> Result<Address> {
        let addr = unix_common::mmap(start, size, strategy)?;

        if !cfg!(feature = "no_mmap_annotation") {
            linux_common::set_vma_name(addr, size, annotation);
        }

        linux_common::set_hugepage(addr, size, strategy.huge_page)?;

        // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)

        Ok(addr)
    }

    fn munmap(start: Address, size: usize) -> Result<()> {
        unix_common::munmap(start, size)
    }

    fn mprotect(start: Address, size: usize) -> Result<()> {
        unix_common::mprotect(start, size)
    }

    fn munprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
        unix_common::munprotect(start, size, prot)
    }

    fn is_mmap_oom(os_errno: i32) -> bool {
        unix_common::is_mmap_oom(os_errno)
    }

    fn panic_if_unmapped(start: Address, size: usize) {
        let strategy = MmapStrategy {
            huge_page: HugePageSupport::No,
            prot: MmapProtection::ReadWrite,
            replace: false,
            reserve: true,
        };
        match unix_common::mmap(start, size, strategy) {
            Ok(_) => panic!("{} of size {} is not mapped", start, size),
            Err(e) => {
                assert!(
                    e.kind() == std::io::ErrorKind::AlreadyExists,
                    "Failed to check mapped: {:?}",
                    e
                );
            }
        }
    }
}

impl OSProcess for Android {
    fn get_process_memory_maps() -> Result<String> {
        linux_common::get_process_memory_maps()
    }

    fn get_process_id() -> Result<String> {
        unix_common::get_process_id()
    }

    fn get_thread_id() -> Result<String> {
        unix_common::get_thread_id()
    }

    fn get_total_num_cpus() -> CoreNum {
        linux_common::get_total_num_cpus()
    }

    fn bind_current_thread_to_core(core_id: CoreId) {
        linux_common::bind_current_thread_to_core(core_id)
    }

    fn bind_current_thread_to_cpuset(core_ids: &[CoreId]) {
        linux_common::bind_current_thread_to_cpuset(core_ids)
    }
}
