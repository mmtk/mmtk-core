use crate::util::os::*;
use crate::util::os::posix_common;
use crate::util::address::Address;
use std::io::Result;

pub struct LinuxMemoryImpl;

impl Memory for LinuxMemoryImpl {
    fn dzmmap(start: Address, size: usize, strategy: MmapStrategy, annotation: &MmapAnnotation<'_>) -> Result<Address> {
        // println!("Mmap with strategy: {:?}", strategy);
        let addr = posix_common::mmap(start, size, strategy)?;
        // println!("Mmap done");

        if !cfg!(feature = "no_mmap_annotation") {
            posix_common::set_vma_name(addr, size, annotation);            
            // println!("Set annotation done");
        }

        Self::set_hugepage(addr, size, strategy.huge_page)?;
        // println!("Set huge page done");

        // Zero memory if needed
        Ok(addr)
    }

    fn munmap(start: Address, size: usize) -> Result<()> {
        posix_common::munmap(start, size)
    }

    fn mprotect(start: Address, size: usize) -> Result<()> {
        posix_common::mprotect(start, size)
    }

    fn munprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
        posix_common::munprotect(start, size, prot)
    }

    fn is_mmap_oom(os_errno: i32) -> bool {
        posix_common::is_mmap_oom(os_errno)
    }

    fn panic_if_unmapped(start: Address, size: usize) {
        let strategy = MmapStrategy {
            huge_page: HugePageSupport::No,
            prot: MmapProtection::ReadWrite,
            replace: false,
            reserve: true,
        };
        match posix_common::mmap(
            start,
            size,
            strategy,
        ) {
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

impl LinuxMemoryImpl {
    pub fn set_hugepage(start: Address, size: usize, options: HugePageSupport) -> Result<()> {
        match options {
            HugePageSupport::No => Ok(()),
            HugePageSupport::TransparentHugePages => {
                    posix_common::wrap_libc_call(
                        &|| unsafe { libc::madvise(start.to_mut_ptr(), size, libc::MADV_HUGEPAGE) },
                        0,
                    )
            }
        }
    }
}

impl MmapStrategy {
    pub fn get_mmap_flags(&self) -> i32 {
        let mut flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS;
        if self.replace {
            flags |= libc::MAP_FIXED;
        } else {
            flags |= libc::MAP_FIXED_NOREPLACE
        }
        if !self.reserve {
            flags |= libc::MAP_NORESERVE;
        }
        flags
    }
}

pub struct LinuxProcessImpl;

impl Process for LinuxProcessImpl {
    fn get_process_memory_maps() -> Result<String> {
        posix_common::get_process_memory_maps()
    }
}
