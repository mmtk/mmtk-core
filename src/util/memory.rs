use crate::util::alloc::AllocationError;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::{Collection, VMBinding};
use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::io::{Error, Result};

pub fn result_is_mapped(result: Result<()>) -> bool {
    match result {
        Ok(_) => false,
        Err(err) => err.raw_os_error().unwrap() == libc::EEXIST,
    }
}

pub fn zero(start: Address, len: usize) {
    set(start, 0, len);
}

pub fn set(start: Address, val: u8, len: usize) {
    unsafe {
        std::ptr::write_bytes::<u8>(start.to_mut_ptr(), val, len);
    }
}

/// Demand-zero mmap:
/// This function mmaps the memory and guarantees to zero all mapped memory.
/// This function WILL overwrite existing memory mapping. The user of this function
/// needs to be aware of this, and use it cautiously.
///
/// # Safety
/// This function WILL overwrite existing memory mapping if there is any. So only use this function if you know
/// the memory has been reserved by mmtk (e.g. after the use of mmap_noreserve()). Otherwise using this function
/// may corrupt others' data.
#[allow(clippy::let_and_return)] // Zeroing is not neceesary for some OS/s
pub unsafe fn dzmmap(start: Address, size: usize) -> Result<()> {
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let ret = mmap_fixed(start, size, prot, flags);
    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)
    #[cfg(not(target_os = "linux"))]
    if ret.is_ok() {
        zero(start, size)
    }
    ret
}

#[cfg(target_os = "linux")]
// MAP_FIXED_NOREPLACE returns EEXIST if already mapped
const MMAP_FLAGS: libc::c_int = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
#[cfg(target_os = "macos")]
// MAP_FIXED is used instead of MAP_FIXED_NOREPLACE (which is not available on macOS). We are at the risk of overwriting pre-existing mappings.
const MMAP_FLAGS: libc::c_int = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;

/// Demand-zero mmap (no replace):
/// This function mmaps the memory and guarantees to zero all mapped memory.
/// This function will not overwrite existing memory mapping, and it will result Err if there is an existing mapping.
#[allow(clippy::let_and_return)] // Zeroing is not neceesary for some OS/s
pub fn dzmmap_noreplace(start: Address, size: usize) -> Result<()> {
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = MMAP_FLAGS;
    let ret = mmap_fixed(start, size, prot, flags);
    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)
    #[cfg(not(target_os = "linux"))]
    if ret.is_ok() {
        zero(start, size)
    }
    ret
}

/// mmap with no swap space reserve:
/// This function does not reserve swap space for this mapping, which means there is no guarantee that writes to the
/// mapping can always be successful. In case of out of physical memory, one may get a segfault for writing to the mapping.
/// We can use this to reserve the address range, and then later overwrites the mapping with dzmmap().
pub fn mmap_noreserve(start: Address, size: usize) -> Result<()> {
    let prot = PROT_NONE;
    let flags = MMAP_FLAGS | libc::MAP_NORESERVE;
    mmap_fixed(start, size, prot, flags)
}

pub fn mmap_fixed(
    start: Address,
    size: usize,
    prot: libc::c_int,
    flags: libc::c_int,
) -> Result<()> {
    let ptr = start.to_mut_ptr();
    wrap_libc_call(
        &|| unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) },
        ptr,
    )
}

pub fn munmap(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0)
}

/// Properly handle errors from a mmap Result, including invoking the binding code in the case of
/// an OOM error.
pub fn handle_mmap_error<VM: VMBinding>(error: Error, tls: VMThread) -> ! {
    use std::io::ErrorKind;

    match error.kind() {
        // From Rust nightly 2021-05-12, we started to see Rust added this ErrorKind.
        ErrorKind::OutOfMemory => {
            // Signal `MmapOutOfMemory`. Expect the VM to abort immediately.
            trace!("Signal MmapOutOfMemory!");
            VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
            unreachable!()
        }
        // Before Rust had ErrorKind::OutOfMemory, this is how we capture OOM from OS calls.
        // TODO: We may be able to remove this now.
        ErrorKind::Other => {
            // further check the error
            if let Some(os_errno) = error.raw_os_error() {
                // If it is OOM, we invoke out_of_memory() through the VM interface.
                if os_errno == libc::ENOMEM {
                    // Signal `MmapOutOfMemory`. Expect the VM to abort immediately.
                    trace!("Signal MmapOutOfMemory!");
                    VM::VMCollection::out_of_memory(tls, AllocationError::MmapOutOfMemory);
                    unreachable!()
                }
            }
        }
        ErrorKind::AlreadyExists => panic!("Failed to mmap, the address is already mapped. Should MMTk quanrantine the address range first?"),
        _ => {}
    }
    panic!("Unexpected mmap failure: {:?}", error)
}

/// Checks if the memory has already been mapped. If not, we panic.
// Note that the checking has a side effect that it will map the memory if it was unmapped. So we panic if it was unmapped.
// Be very careful about using this function.
#[cfg(target_os = "linux")]
pub fn panic_if_unmapped(start: Address, size: usize) {
    let prot = PROT_READ | PROT_WRITE;
    let flags = MMAP_FLAGS;
    match mmap_fixed(start, size, prot, flags) {
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

#[cfg(not(target_os = "linux"))]
pub fn panic_if_unmapped(_start: Address, _size: usize) {
    // This is only used for assertions, so MMTk will still run even if we never panic.
    // TODO: We need a proper implementation for this. As we do not have MAP_FIXED_NOREPLACE, we cannot use the same implementation as Linux.
    // Possibly we can use posix_mem_offset for both OS/s.
}

pub fn munprotect(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_READ | PROT_WRITE | PROT_EXEC) },
        0,
    )
}

pub fn mprotect(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_NONE) },
        0,
    )
}

fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Get the memory maps for the process. The returned string is a multi-line string.
/// This is only meant to be used for debugging. For example, log process memory maps after detecting a clash.
/// If we would need to parsable memory maps, I would suggest using a library instead which saves us the trouble to deal with portability.
#[cfg(debug_assertions)]
#[cfg(target_os = "linux")]
pub fn get_process_memory_maps() -> String {
    // print map
    use std::fs::File;
    use std::io::Read;
    let mut data = String::new();
    let mut f = File::open("/proc/self/maps").unwrap();
    f.read_to_string(&mut data).unwrap();
    data
}

/// Returns the total physical memory for the system in bytes.
pub(crate) fn get_system_total_memory() -> usize {
    match sys_info::mem_info() {
        Ok(mem_info) => mem_info.total as usize,
        Err(e) => {
            warn!(
                "Failed to get sys_info::mem_info: {:?}. Return 1G in get_system_total_memory()",
                e
            );
            1024 * 1024 * 1024
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::constants::BYTES_IN_PAGE;
    use crate::util::test_util::MEMORY_TEST_REGION;
    use crate::util::test_util::{serial_test, with_cleanup};

    // In the tests, we will mmap this address. This address should not be in our heap (in case we mess up with other tests)
    const START: Address = MEMORY_TEST_REGION.start;

    #[test]
    fn test_mmap() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = unsafe { dzmmap(START, BYTES_IN_PAGE) };
                    assert!(res.is_ok());
                    // We can overwrite with dzmmap
                    let res = unsafe { dzmmap(START, BYTES_IN_PAGE) };
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            );
        });
    }

    #[test]
    fn test_munmap() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = dzmmap_noreplace(START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                    let res = munmap(START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_mmap_noreplace() {
        serial_test(|| {
            with_cleanup(
                || {
                    // Make sure we mmapped the memory
                    let res = unsafe { dzmmap(START, BYTES_IN_PAGE) };
                    assert!(res.is_ok());
                    // Use dzmmap_noreplace will fail
                    let res = dzmmap_noreplace(START, BYTES_IN_PAGE);
                    assert!(res.is_err());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        });
    }

    #[test]
    fn test_mmap_noreserve() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = mmap_noreserve(START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                    // Try reserve it
                    let res = unsafe { dzmmap(START, BYTES_IN_PAGE) };
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[should_panic]
    fn test_check_is_mmapped_for_unmapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We expect this call to panic
                    panic_if_unmapped(START, BYTES_IN_PAGE);
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_check_is_mmapped_for_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    assert!(dzmmap_noreplace(START, BYTES_IN_PAGE).is_ok());
                    panic_if_unmapped(START, BYTES_IN_PAGE);
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[should_panic]
    fn test_check_is_mmapped_for_unmapped_next_to_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    // map 1 page from START
                    assert!(dzmmap_noreplace(START, BYTES_IN_PAGE).is_ok());

                    // check if the next page is mapped - which should panic
                    panic_if_unmapped(START + BYTES_IN_PAGE, BYTES_IN_PAGE);
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE * 2).is_ok());
                },
            )
        })
    }

    #[test]
    #[should_panic]
    // This is a bug we need to fix. We need to figure out a way to properly check if a piece of memory is mapped or not.
    // Alternatively, we should remove the code that calls the function.
    #[ignore]
    fn test_check_is_mmapped_for_partial_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    // map 1 page from START
                    assert!(dzmmap_noreplace(START, BYTES_IN_PAGE).is_ok());

                    // check if the 2 pages from START are mapped. The second page is unmapped, so it should panic.
                    panic_if_unmapped(START, BYTES_IN_PAGE * 2);
                },
                || {
                    assert!(munmap(START, BYTES_IN_PAGE * 2).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_get_system_total_memory() {
        let total = get_system_total_memory();
        println!("Total memory: {:?}", total);
    }
}
