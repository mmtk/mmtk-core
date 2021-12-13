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
    let ptr = start.to_mut_ptr();
    wrap_libc_call(&|| unsafe { libc::memset(ptr, 0, len) }, ptr).unwrap()
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

/// Demand-zero mmap (no replace):
/// This function mmaps the memory and guarantees to zero all mapped memory.
/// This function will not overwrite existing memory mapping, and it will result Err if there is an existing mapping.
#[allow(clippy::let_and_return)] // Zeroing is not neceesary for some OS/s
pub fn dzmmap_noreplace(start: Address, size: usize) -> Result<()> {
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
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
    let flags =
        libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE | libc::MAP_NORESERVE;
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

/// Properly handle errors from a mmap Result, including invoking the binding code for an OOM error.
pub fn handle_mmap_error<VM: VMBinding>(error: Error, tls: VMThread) -> ! {
    use std::io::ErrorKind;

    match error.kind() {
        // From Rust nightly 2021-05-12, we started to see Rust added this ErrorKind.
        // ErrorKind::OutOfMemory => {
        //     VM::VMCollection::out_of_memory(tls);
        //     unreachable!()
        // }
        // Before Rust had ErrorKind::OutOfMemory, this is how we capture OOM from OS calls.
        // TODO: We may be able to remove this now.
        ErrorKind::Other => {
            // further check the error
            if let Some(os_errno) = error.raw_os_error() {
                // If it is OOM, we invoke out_of_memory() through the VM interface.
                if os_errno == libc::ENOMEM {
                    VM::VMCollection::out_of_memory(tls);
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
pub fn panic_if_unmapped(start: Address, size: usize) {
    let prot = PROT_READ | PROT_WRITE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
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
}
