use crate::util::address::Address;
use crate::util::os::imp::unix_like::linux_like::linux_common;
use crate::util::os::imp::unix_like::unix_common;
use crate::util::os::*;

use std::io::Result;

/// Linux implementation of the `OS` trait.
pub struct Linux;

impl OSMemory for Linux {
    fn dzmmap(
        start: Address,
        size: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> Result<Address> {
        linux_common::dzmmap(start, size, strategy, annotation)
    }

    fn mmap_noreserve_anywhere(
        size: usize,
        align: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> Result<Address> {
        let addr = unix_common::mmap_anywhere(
            size,
            align,
            strategy.prot(MmapProtection::NoAccess).reserve(false),
        )?;
        if !cfg!(feature = "no_mmap_annotation") {
            linux_common::set_vma_name(addr, size, annotation);
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

    fn panic_if_unmapped(start: Address, size: usize) {
        linux_common::panic_if_unmapped(start, size)
    }
}

impl OSProcess for Linux {
    type ProcessIDType = unix_common::ProcessIDType;
    type ThreadIDType = unix_common::ThreadIDType;

    fn get_process_memory_maps() -> Result<String> {
        linux_common::get_process_memory_maps()
    }

    fn get_process_id() -> Result<Self::ProcessIDType> {
        unix_common::get_process_id()
    }

    fn get_thread_id() -> Result<Self::ThreadIDType> {
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
