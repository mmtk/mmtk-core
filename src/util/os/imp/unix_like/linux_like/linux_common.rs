use crate::util::address::Address;
use crate::util::os::imp::unix_like::unix_common;
use crate::util::os::*;
use libc::{cpu_set_t, sched_getaffinity, sched_setaffinity, CPU_COUNT, CPU_SET, CPU_ZERO};
use std::io::Result;

pub fn set_vma_name(start: Address, size: usize, annotation: &MmapAnnotation) {
    // `PR_SET_VMA` is new in Linux 5.17.  We compile against a version of the `libc` crate that
    // has the `PR_SET_VMA_ANON_NAME` constant.  When runnning on an older kernel, it will not
    // recognize this attribute and will return `EINVAL`.  However, `prctl` may return `EINVAL`
    // for other reasons, too.  That includes `start` being an invalid address, and the
    // formatted `anno_cstr` being longer than 80 bytes including the trailing `'\0'`.  But
    // since this prctl is used for debugging, we log the error instead of panicking.
    let anno_str = annotation.to_string();
    let anno_cstr = std::ffi::CString::new(anno_str).unwrap();
    let result = unix_common::wrap_libc_call(
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

/// Set huge page option for the given memory.
pub fn set_hugepage(start: Address, size: usize, options: HugePageSupport) -> Result<()> {
    match options {
        HugePageSupport::No => Ok(()),
        HugePageSupport::TransparentHugePages => unix_common::wrap_libc_call(
            &|| unsafe { libc::madvise(start.to_mut_ptr(), size, libc::MADV_HUGEPAGE) },
            0,
        ),
    }
}

impl MmapStrategy {
    /// get the flags for POSIX mmap.
    pub fn get_posix_mmap_flags(&self) -> i32 {
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

/// Get the memory maps for the process. The returned string is a multi-line string.
/// This is only meant to be used for debugging. For example, log process memory maps after detecting a clash.
pub fn get_process_memory_maps() -> Result<String> {
    // print map
    use std::fs::File;
    use std::io::Read;
    let mut data = String::new();
    let mut f = File::open("/proc/self/maps")?;
    f.read_to_string(&mut data)?;
    Ok(data)
}

pub fn get_total_num_cpus() -> CoreNum {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        sched_getaffinity(0, std::mem::size_of::<cpu_set_t>(), &mut cs);
        CPU_COUNT(&cs) as u16
    }
}

pub fn bind_current_thread_to_core(core_id: CoreId) {
    use std::mem::MaybeUninit;
    unsafe {
        let mut cs = MaybeUninit::zeroed().assume_init();
        CPU_ZERO(&mut cs);
        CPU_SET(core_id as usize, &mut cs);
        sched_setaffinity(0, std::mem::size_of::<cpu_set_t>(), &cs);
    }
}

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

pub fn dzmmap(
    start: Address,
    size: usize,
    strategy: MmapStrategy,
    annotation: &MmapAnnotation<'_>,
) -> Result<Address> {
    let addr = unix_common::mmap(start, size, strategy)?;

    if !cfg!(feature = "no_mmap_annotation") {
        set_vma_name(addr, size, annotation);
    }

    set_hugepage(addr, size, strategy.huge_page)?;

    // We do not need to explicitly zero for Linux (memory is guaranteed to be zeroed)

    Ok(addr)
}

pub fn panic_if_unmapped(start: Address, size: usize) {
    let strategy = MmapStrategy {
        huge_page: HugePageSupport::No,
        prot: MmapProtection::ReadWrite,
        replace: false,
        reserve: true,
    };
    match unix_common::mmap(start, size, strategy) {
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
