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

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn set_vma_name(start: Address, size: usize, annotation: &MmapAnnotation) {
    // `PR_SET_VMA` is new in Linux 5.17.  We compile against a version of the `libc` crate that
    // has the `PR_SET_VMA_ANON_NAME` constant.  When runnning on an older kernel, it will not
    // recognize this attribute and will return `EINVAL`.  However, `prctl` may return `EINVAL`
    // for other reasons, too.  That includes `start` being an invalid address, and the
    // formatted `anno_cstr` being longer than 80 bytes including the trailing `'\0'`.  But
    // since this prctl is used for debugging, we log the error instead of panicking.
    let anno_str = annotation.to_string();
    let anno_cstr = std::ffi::CString::new(anno_str).unwrap();
    let result = wrap_libc_call(
        &|| unsafe {
            libc::prctl(
                libc::PR_SET_VMA,
                libc::PR_SET_VMA_ANON_NAME,
                start.to_ptr::<libc::c_void>(),
                size,
                anno_cstr.as_ptr(),
            )
        },
        0,
    );
    if let Err(e) = result {
        debug!("Error while calling prctl: {e}");
    }
}

/// Get the memory maps for the process. The returned string is a multi-line string.
/// This is only meant to be used for debugging. For example, log process memory maps after detecting a clash.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn get_process_memory_maps() -> Result<String> {
    // print map
    use std::fs::File;
    use std::io::Read;
    let mut data = String::new();
    let mut f = File::open("/proc/self/maps")?;
    f.read_to_string(&mut data)?;
    Ok(data)
}

pub fn munmap(start: Address, size: usize) -> Result<()> {
    wrap_libc_call(&|| unsafe { libc::munmap(start.to_mut_ptr(), size) }, 0)
}

pub fn mprotect(start: Address, size: usize) -> Result<()> {
    let prot = libc::PROT_NONE;
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, prot) },
        0,
    )
}

pub fn munprotect(start: Address, size: usize, prot: MmapProtection) -> Result<()> {
    wrap_libc_call(
        &|| unsafe { libc::mprotect(start.to_mut_ptr(), size, prot.get_native_flags()) },
        0,
    )
}

pub fn get_process_id() -> Result<String> {
    let pid = unsafe { libc::getpid() };
    Ok(format!("{}", pid))
}

pub fn get_thread_id() -> Result<String> {
    let tid = unsafe { libc::pthread_self() };
    Ok(format!("{}", tid))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
use libc::{cpu_set_t, sched_getaffinity, sched_setaffinity, CPU_COUNT, CPU_SET, CPU_ZERO};

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn get_total_num_cpus() -> CoreNum {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        sched_getaffinity(0, std::mem::size_of::<cpu_set_t>(), &mut cs);
        CPU_COUNT(&cs) as u16
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn bind_current_thread_to_core(core_id: CoreId) {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        CPU_SET(core_id as usize, &mut cs);
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn bind_current_thread_to_cpuset(cpuset: &[CoreId]) {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        for cpu in cpuset {
            CPU_SET(*cpu as usize, &mut cs);
        }
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

pub fn wrap_libc_call<T: PartialEq>(f: &dyn Fn() -> T, expect: T) -> Result<()> {
    let ret = f();
    if ret == expect {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
