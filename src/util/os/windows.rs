use crate::util::os::memory::*;
use crate::util::address::Address;
use std::io::Result;

pub struct WindowsMemoryImpl;

impl Memory for WindowsMemoryImpl {
    fn dzmmap(start: Address, size: usize, strategy: MmapStrategy, _annotation: &MmapAnnotation<'_>) -> Result<Address> {
        use std::io;
        use windows_sys::Win32::System::Memory::{
            VirtualAlloc, VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_FREE, MEM_RESERVE,
        };

        let ptr: *mut u8 = start.to_mut_ptr();

        // Has to COMMIT immediately if:
        // - not MAP_NORESERVE
        // - and protection is not NoAccess
        let commit = strategy.reserve && !matches!(strategy.prot, MmapProtection::NoAccess);

        // Scan the region [ptr, ptr + size) to understand its current state
        unsafe {
            let mut addr = ptr;
            let end = ptr.add(size);

            let mut saw_free = false;
            let mut saw_reserved = false;
            let mut saw_committed = false;

            while addr < end {
                let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
                let q = VirtualQuery(
                    addr as *const _,
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                );
                if q == 0 {
                    return Err(io::Error::last_os_error());
                }

                let region_base = mbi.BaseAddress as *mut u8;
                let region_size = mbi.RegionSize;
                let region_end = region_base.add(region_size);

                // Calculate the intersection of [addr, end) and [region_base, region_end)
                let _sub_begin = if addr > region_base {
                    addr
                } else {
                    region_base
                };
                let _sub_end = if end < region_end { end } else { region_end };

                match mbi.State {
                    MEM_FREE => saw_free = true,
                    MEM_RESERVE => saw_reserved = true,
                    MEM_COMMIT => saw_committed = true,
                    _ => {
                        return Err(io::Error::other("Unexpected memory state in mmap_fixed"));
                    }
                }

                // Jump to the next region (VirtualQuery always returns "continuous regions with the same attributes")
                addr = region_end;
            }

            // 1. All FREE: make a new mapping in the region
            // 2. All RESERVE/COMMIT: treat as an existing mapping, can just COMMIT or succeed directly
            // 3. MIX of FREE + others: not allowed (semantically similar to MAP_FIXED_NOREPLACE)
            if saw_free && (saw_reserved || saw_committed) {
                return Err(io::Error::from_raw_os_error(
                    windows_sys::Win32::Foundation::ERROR_INVALID_ADDRESS as i32,
                ));
            }

            if saw_free && !saw_reserved && !saw_committed {
                // All FREE: make a new mapping in the region
                let mut allocation_type = MEM_RESERVE;
                if commit {
                    allocation_type |= MEM_COMMIT;
                }

                let res = VirtualAlloc(ptr as *mut _, size, allocation_type, strategy.prot.into_native_flags());
                if res.is_null() {
                    return Err(io::Error::last_os_error());
                }

                Ok(start)
            } else {
                // This behavior is similar to mmap with MAP_FIXED on Linux.
                // If the region is already mapped, we just ensure the required commitment.
                // If commit is not needed, we just return Ok.
                if commit {
                    let res = VirtualAlloc(ptr as *mut _, size, MEM_COMMIT, strategy.prot.into_native_flags());
                    if res.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(start)
            }
        }
    }

    fn munmap(start: Address, size: usize) -> Result<()> {
        use windows_sys::Win32::System::Memory::*;
        // Using MEM_DECOMMIT will decommit the memory but leave the address space reserved.
        // This is the safest way to emulate munmap on Windows, as MEM_RELEASE would free
        // the entire allocation, which could be larger than the requested size.
        let res = unsafe { VirtualFree(start.to_mut_ptr(), size, MEM_DECOMMIT) };
        if res == 0 {
            // If decommit fails, we try to release the memory. This might happen if the memory was
            // only reserved.
            let res_release = unsafe { VirtualFree(start.to_mut_ptr(), 0, MEM_RELEASE) };
            if res_release == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    fn mprotect(start: Address, size: usize) -> Result<()> {
        use windows_sys::Win32::System::Memory::*;
        let mut old_protect = 0;
        let res =
            unsafe { VirtualProtect(start.to_mut_ptr(), size, PAGE_NOACCESS, &mut old_protect) };
        if res == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn munprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
        use windows_sys::Win32::System::Memory::*;
        let prot = prot.into_native_flags();
        let mut old_protect = 0;
        let res = unsafe { VirtualProtect(start.to_mut_ptr(), size, prot, &mut old_protect) };
        if res == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn panic_if_unmapped(start: Address, size: usize, _annotation: &MmapAnnotation<'_>) {
        warn!("Check if {} of size {} is mapped is ignored on Windows", start, size);
    }

    fn is_mmap_oom(os_errno: i32) -> bool {
        os_errno == windows_sys::Win32::Foundation::ERROR_NOT_ENOUGH_MEMORY as i32 || os_errno == windows_sys::Win32::Foundation::ERROR_INVALID_ADDRESS as i32
    }
}

impl MmapProtection {
    fn into_native_flags(&self) -> u32 {
        use windows_sys::Win32::System::Memory::*;
        match self {
            Self::ReadWrite => PAGE_READWRITE,
            Self::ReadWriteExec => PAGE_EXECUTE_READWRITE,
            Self::NoAccess => PAGE_NOACCESS,
        }
    }
}

use crate::util::os::process::*;

pub struct WindowsProcessImpl;

impl Process for WindowsProcessImpl {
    fn get_process_memory_maps() -> Result<String> {
        // Windows-specific implementation to get process memory maps
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "get_process_memory_maps not implemented for Windows",
        ))
    }
}
