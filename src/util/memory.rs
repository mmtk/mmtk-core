use crate::util::Address;
use libc::{c_void, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use std::io::{Error, Result};

pub fn zero(start: Address, len: usize) {
    unsafe {
        libc::memset(start.to_mut_ptr() as *mut libc::c_void, 0, len);
    }
}

/// Demand-zero mmap:
/// This function guarantees to zero all mapped memory.
pub fn dzmmap(start: Address, size: usize) -> Result<Address> {
    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let result: *mut c_void = unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) };
    let addr = Address::from_mut_ptr(result);
    if addr == start {
        // On linux, we don't need to zero the memory. This is achieved by using the `MAP_ANON` mmap flag.
        #[cfg(not(target_os = "linux"))]
        {
            zero(addr, size);
        }
        Ok(addr)
    } else {
        assert!(result as usize <= 127,
                "mmap with MAP_FIXED has unexpected behavior: demand zero mmap with MAP_FIXED on {:?} returned some other address {:?}",
                start, result
        );
        Err(Error::from_raw_os_error(result as _))
    }
}

pub fn munprotect(start: Address, size: usize) -> Result<()> {
    let result =
        unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_READ | PROT_WRITE | PROT_EXEC) };
    if result == 0 {
        Ok(())
    } else {
        Err(Error::from_raw_os_error(result))
    }
}

pub fn mprotect(start: Address, size: usize) -> Result<()> {
    let result = unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_NONE) };
    if result == 0 {
        Ok(())
    } else {
        Err(Error::from_raw_os_error(result))
    }
}

/// Reserves a contiguous virtual address range with the specified size,
/// without mapping it to physical memory.
/// The dzmmap function can then be used to map specific portions of the address range to physical memory.
/// 
pub fn reserve_vm_address_range(size: usize) -> Result<Address> {
    let prot = libc::PROT_NONE;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE;
    let result: *mut c_void = unsafe { libc::mmap(std::ptr::null_mut(), size, prot, flags, -1, 0) };
    if result == libc::MAP_FAILED {
        Err(Error::from_raw_os_error(result))
    } else {
        Ok(Address::from_mut_ptr(result))
    }
}
