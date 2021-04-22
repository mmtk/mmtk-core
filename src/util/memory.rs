use crate::util::Address;
use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::io::Result;

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
/// This function guarantees to zero all mapped memory.
pub fn dzmmap(start: Address, size: usize) -> Result<()> {
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let ret = mmap_fixed(start, size, prot, flags);
    if ret.is_ok() {
        #[cfg(not(target_os = "linux"))]
        zero(start, size)
    }
    ret
}

/// Demand-zero mmap:
/// This function guarantees to zero all mapped memory.
/// FIXME - this function should replace dzmmap.
/// Currently, the replacement causes some of the concurrent tests to fail
pub fn dzmmap_noreplace(start: Address, size: usize) -> Result<()> {
    let prot = PROT_READ | PROT_WRITE | PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
    let ret = mmap_fixed(start, size, prot, flags);
    if ret.is_ok() {
        #[cfg(not(target_os = "linux"))]
        zero(start, size)
    }
    ret
}

/// mmap with no swap space reserve:
/// This function only maps the address range, but doesn't occupy any physical memory.
///
/// Before using any part of the address range, dzmmap must be called.
///
pub fn mmap_noreserve(start: Address, size: usize) -> Result<()> {
    let prot = PROT_READ | PROT_WRITE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
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
            println!("{:?}", e);
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
    use crate::util::heap::layout::vm_layout_constants::HEAP_START;
    use crate::util::test_util::{serial_test, with_cleanup};

    #[test]
    fn test_mmap() {
        serial_test(|| {
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());
            // We can overwrite with dzmmap
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());

            assert!(munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
        });
    }

    #[test]
    fn test_munmap() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                    let res = munmap(HEAP_START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
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
                    let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                    // Use dzmmap_noreplace will fail
                    let res = dzmmap_noreplace(HEAP_START, BYTES_IN_PAGE);
                    println!("{:?}", res);
                    assert!(res.is_err());
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
                },
            )
        });
    }

    #[test]
    fn test_mmap_noreserve() {
        serial_test(|| {
            with_cleanup(
                || {
                    let res = mmap_noreserve(HEAP_START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                    unsafe {
                        HEAP_START.store(42usize);
                    }
                    // Try reserve it
                    let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
                    assert!(res.is_ok());
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
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
                    panic_if_unmapped(HEAP_START, BYTES_IN_PAGE);
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
                },
            )
        })
    }

    #[test]
    fn test_check_is_mmapped_for_mapped() {
        serial_test(|| {
            with_cleanup(
                || {
                    assert!(dzmmap(HEAP_START, BYTES_IN_PAGE).is_ok());
                    panic_if_unmapped(HEAP_START, BYTES_IN_PAGE);
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
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
                    // map 1 page from HEAP_START
                    assert!(dzmmap(HEAP_START, BYTES_IN_PAGE).is_ok());

                    // check if the next page is mapped - which should panic
                    panic_if_unmapped(HEAP_START + BYTES_IN_PAGE, BYTES_IN_PAGE);
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE * 2).is_ok());
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
                    // map 1 page from HEAP_START
                    assert!(dzmmap(HEAP_START, BYTES_IN_PAGE).is_ok());

                    // check if the 2 pages from HEAP_START are mapped. The second page is unmapped, so it should panic.
                    panic_if_unmapped(HEAP_START, BYTES_IN_PAGE * 2);
                },
                || {
                    assert!(munmap(HEAP_START, BYTES_IN_PAGE * 2).is_ok());
                },
            )
        })
    }
}
