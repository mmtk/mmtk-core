use crate::util::Address;
use std::io::{Result, Error};
use libc::{PROT_READ, PROT_WRITE, PROT_EXEC, PROT_NONE, c_void};

pub fn zero(start: Address, len: usize) {
    unsafe {
        libc::memset(start.to_mut_ptr() as *mut libc::c_void, 0, len);
    }
}

pub fn dzmmap(start: Address, size: usize) -> Result<Address> {
    let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED;
    let result: *mut c_void = unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) };
    if Address::from_mut_ptr(result) == start {
        Ok(Address::from_mut_ptr(result))
    } else {
        assert!(result as usize <= 127,
                "mmap with MAP_FIXED has unexpected behavior: demand zero mmap with MAP_FIXED on {:?} returned some other address {:?}",
                start, result
        );
        Err(Error::from_raw_os_error(result as _))
    }
}

pub fn munprotect(start: Address, size: usize) -> Result<()> {
    let result = unsafe { libc::mprotect(start.to_mut_ptr(), size, PROT_READ | PROT_WRITE | PROT_EXEC) };
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
