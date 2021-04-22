use crate::util::Address;
use libc::{c_void, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::io::{Error, ErrorKind, Result};

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
    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
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
    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
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
    let prot = libc::PROT_READ | libc::PROT_WRITE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags =
        libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE | libc::MAP_NORESERVE;
    mmap_fixed(start, size, prot, flags)
}

fn mmap_fixed(start: Address, size: usize, prot: libc::c_int, flags: libc::c_int) -> Result<()> {
    let ptr = start.to_mut_ptr();
    wrap_libc_call(&|| unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) }, ptr)
}

pub fn munprotect(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_READ | PROT_WRITE | PROT_EXEC) }, 0)
}

pub fn mprotect(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_NONE) }, 0)
}

fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}


pub fn try_munmap(start: Address, size: usize) -> Result<()> {
    let result = unsafe { libc::munmap(start.to_mut_ptr(), size) };
    if result == -1 {
        let err = unsafe { *libc::__errno_location() };
        Err(Error::from_raw_os_error(err as _))
    } else {
        Ok(())
    }
}

//
pub fn check_is_mmapped(start: Address, size: usize) -> Result<()> {
    let prot = libc::PROT_READ | libc::PROT_WRITE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;

    let result: *mut libc::c_void =
        unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) };

    if result != libc::MAP_FAILED {
        return Err(Error::new(ErrorKind::InvalidInput, "NotMMapped"));
    }

    let err = unsafe { *libc::__errno_location() };
    if err == libc::EEXIST {
        Ok(())
    } else {
        Err(Error::from_raw_os_error(err as _))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::heap::layout::vm_layout_constants::HEAP_START;
    use crate::util::constants::BYTES_IN_PAGE;
    use crate::util::test_util::serial_test;

    #[test]
    fn test_mmap() {
        serial_test(|| {
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());
            // We can overwrite with dzmmap
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());

            assert!(try_munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
        });
    }

    #[test]
    fn test_try_munmap() {
        serial_test(|| {
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());
            let res = try_munmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());

            assert!(try_munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
        })
    }

    #[test]
    fn test_mmap_noreplace() {
        serial_test(|| {
            // Make sure we mmapped the memory
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());
            // Use dzmmap_noreplace will fail
            let res = dzmmap_noreplace(HEAP_START, BYTES_IN_PAGE);
            println!("{:?}", res);
            assert!(res.is_err());

            assert!(try_munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
        });
    }

    #[test]
    fn test_mmap_noreserve() {
        serial_test(|| {
            let res = mmap_noreserve(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());
            unsafe { HEAP_START.store(42usize); }
            // Try reserve it
            let res = dzmmap(HEAP_START, BYTES_IN_PAGE);
            assert!(res.is_ok());

            assert!(try_munmap(HEAP_START, BYTES_IN_PAGE).is_ok());
        })
    }
}