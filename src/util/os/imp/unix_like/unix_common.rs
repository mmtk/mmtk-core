use crate::util::address::Address;
use crate::util::os::*;
use std::io::Result;

impl MmapProtection {
    fn get_native_flags(&self) -> i32 {
        use libc::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
        match self {
            Self::ReadWrite => PROT_READ | PROT_WRITE,
            Self::ReadWriteExec => PROT_READ | PROT_WRITE | PROT_EXEC,
            Self::NoAccess => PROT_NONE,
        }
    }
}

pub fn mmap(start: Address, size: usize, strategy: MmapStrategy) -> Result<Address> {
    let ptr = start.to_mut_ptr();
    let prot = strategy.prot.get_native_flags();
    let flags = strategy.get_posix_mmap_flags();
    wrap_libc_call(
        &|| unsafe { libc::mmap(start.to_mut_ptr(), size, prot, flags, -1, 0) },
        ptr,
    )?;
    Ok(start)
}

pub fn is_mmap_oom(os_errno: i32) -> bool {
    os_errno == libc::ENOMEM
}

pub fn munmap(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0)
}

pub fn mprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, prot.get_native_flags()) },
        0,
    )
}

pub type ProcessIDType = libc::pid_t;
pub type ThreadIDType = libc::pthread_t;

pub fn get_process_id() -> Result<ProcessIDType> {
    Ok(unsafe { libc::getpid() })
}

pub fn get_thread_id() -> Result<ThreadIDType> {
    Ok(unsafe { libc::pthread_self() })
}

pub fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
